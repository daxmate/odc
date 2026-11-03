import Foundation

/// Result of verifying a detached signature sidecar.
///
/// Cryptographic failures are business results, not thrown errors: the FFI
/// returns status 0 with `valid: false` and an `ODC-E0xx` code, so the UI can
/// show the reason without treating it as a broken call.
public struct SignatureVerification: Equatable, Sendable {
    /// True when the signature matches the file and the key is acceptable.
    public let valid: Bool
    /// Lower-case hex of the key that produced the signature.
    public let publicKeyHex: String?
    /// Name the publisher put into the sidecar, when there is one.
    public let signer: String?
    /// Timestamp string from the sidecar, shown as-is.
    public let createdAt: String?
    /// BLAKE3 digest recorded by the signature.
    public let hash: String?
    /// Size of the signed file, in bytes.
    public let size: Int64?
    /// `ODC-E0xx` code for an invalid signature.
    public let errorCode: String?
    /// Human readable reason for an invalid signature.
    public let error: String?

    public init(
        valid: Bool,
        publicKeyHex: String? = nil,
        signer: String? = nil,
        createdAt: String? = nil,
        hash: String? = nil,
        size: Int64? = nil,
        errorCode: String? = nil,
        error: String? = nil
    ) {
        self.valid = valid
        self.publicKeyHex = publicKeyHex
        self.signer = signer
        self.createdAt = createdAt
        self.hash = hash
        self.size = size
        self.errorCode = errorCode
        self.error = error
    }

    /// Name to show for the signer; the sidecar may leave it empty.
    public var displayName: String {
        guard let signer, !signer.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            return "未命名发布者"
        }
        return signer
    }

    /// Short fingerprint of the signing key, or nil when the sidecar has none.
    public var fingerprint: String? {
        guard let publicKeyHex else { return nil }
        let normalised = PublisherIdentity.normalizedPublicKeyHex(publicKeyHex)
        guard PublisherIdentity.isValidPublicKeyHex(normalised) else { return nil }
        return PublisherIdentity(name: "", publicKeyHex: normalised).fingerprint
    }

    /// One line describing why verification failed.
    public var failureText: String {
        switch (errorCode, error) {
        case let (code?, message?): return "\(code) \(message)"
        case let (code?, nil): return code
        case let (nil, message?): return message
        case (nil, nil): return "签名无效。"
        }
    }
}

extension SignatureVerification: Decodable {
    private enum CodingKeys: String, CodingKey {
        case valid
        case publicKeyHex = "public_key"
        case signer
        case createdAt = "created_at"
        case hash
        case size
        case errorCode = "error_code"
        case error
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        valid = try container.decode(Bool.self, forKey: .valid)
        publicKeyHex = try container.decodeIfPresent(String.self, forKey: .publicKeyHex)
        signer = try container.decodeIfPresent(String.self, forKey: .signer)
        createdAt = SignatureVerification.flexibleText(container, .createdAt)
        hash = try container.decodeIfPresent(String.self, forKey: .hash)
        size = try container.decodeIfPresent(Int64.self, forKey: .size)
        errorCode = try container.decodeIfPresent(String.self, forKey: .errorCode)
        error = try container.decodeIfPresent(String.self, forKey: .error)
    }

    /// The core writes some timestamps as strings and older builds as epoch
    /// seconds; accept both instead of failing the whole decode.
    private static func flexibleText(
        _ container: KeyedDecodingContainer<CodingKeys>,
        _ key: CodingKeys
    ) -> String? {
        if let text = try? container.decodeIfPresent(String.self, forKey: key) {
            return text
        }
        if let seconds = try? container.decodeIfPresent(Int64.self, forKey: key) {
            return String(seconds)
        }
        return nil
    }
}

/// Report returned by `odc_patch_apply_signed`.
public struct PatchApplyReport: Decodable, Equatable, Sendable {
    /// True when the patch landed.
    public let applied: Bool
    /// True when the patch went through the three-way merge path.
    public let merged: Bool
    /// Hex of the commit the patch produced.
    public let target: String?
    public let baseBlocks: Int?
    public let newBlocks: Int?
    public let indexChanges: Int?
    public let bytes: Int64?
    /// Number of conflicts recorded by the merge; they stay in the repository
    /// for the existing conflict workflow and are never resolved here.
    public let conflicts: Int

    public init(
        applied: Bool,
        merged: Bool,
        target: String? = nil,
        baseBlocks: Int? = nil,
        newBlocks: Int? = nil,
        indexChanges: Int? = nil,
        bytes: Int64? = nil,
        conflicts: Int = 0
    ) {
        self.applied = applied
        self.merged = merged
        self.target = target
        self.baseBlocks = baseBlocks
        self.newBlocks = newBlocks
        self.indexChanges = indexChanges
        self.bytes = bytes
        self.conflicts = conflicts
    }

    /// One line for the transient summary under the entry.
    public var summaryText: String {
        var text = merged ? "已合并更新" : "已应用更新"
        if let newBlocks {
            text += "，新增块 \(newBlocks)"
        }
        if let indexChanges {
            text += "，索引变更 \(indexChanges)"
        }
        if conflicts > 0 {
            text += "，\(conflicts) 处冲突未解决"
        }
        return text + "。"
    }

    private enum CodingKeys: String, CodingKey {
        case applied
        case merged
        case target
        case baseBlocks = "base_blocks"
        case newBlocks = "new_blocks"
        case indexChanges = "index_changes"
        case bytes
        case conflicts
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        applied = try container.decode(Bool.self, forKey: .applied)
        merged = try container.decodeIfPresent(Bool.self, forKey: .merged) ?? false
        target = try container.decodeIfPresent(String.self, forKey: .target)
        baseBlocks = try container.decodeIfPresent(Int.self, forKey: .baseBlocks)
        newBlocks = try container.decodeIfPresent(Int.self, forKey: .newBlocks)
        indexChanges = try container.decodeIfPresent(Int.self, forKey: .indexChanges)
        bytes = try container.decodeIfPresent(Int64.self, forKey: .bytes)
        conflicts = try container.decodeIfPresent(Int.self, forKey: .conflicts) ?? 0
    }
}

/// Detached signatures live next to their artifact as `<file>.sig`.
public enum ReleaseSidecar {
    /// Suffix appended to the artifact name.
    public static let fileExtension = "sig"

    /// `/x/foo.odc` -> `/x/foo.odc.sig`.
    public static func signatureURL(for artifact: URL) -> URL {
        artifact.appendingPathExtension(fileExtension)
    }

    /// True when the sidecar exists and is a regular file.
    public static func hasSignature(for artifact: URL) -> Bool {
        var isDirectory: ObjCBool = false
        let exists = FileManager.default.fileExists(
            atPath: signatureURL(for: artifact).path,
            isDirectory: &isDirectory
        )
        return exists && !isDirectory.boolValue
    }
}

/// What the trust store says about a verified signature.
public enum TrustDecision: Equatable, Sendable {
    /// The key is already trusted; the stored publisher is attached for display.
    case trusted(Publisher)
    /// Signature is valid and the trust store is empty: ask before trusting.
    case firstTrust
    /// Signature is valid but the key is unknown to a non-empty store.
    case unknownPublisher(String)
    /// The signature itself does not verify.
    case invalidSignature(String)
    /// The sidecar did not carry a usable public key.
    case missingPublicKey

    /// Classify `verification` against the repository's trusted publishers.
    ///
    /// The order matters: an invalid signature is rejected before the trust
    /// store is consulted, so an empty store never silently trusts bad bytes.
    public static func evaluate(
        verification: SignatureVerification,
        publishers: [Publisher]
    ) -> TrustDecision {
        guard verification.valid else {
            return .invalidSignature(verification.failureText)
        }
        guard let rawKey = verification.publicKeyHex else {
            return .missingPublicKey
        }
        let key = PublisherIdentity.normalizedPublicKeyHex(rawKey)
        guard PublisherIdentity.isValidPublicKeyHex(key) else {
            return .missingPublicKey
        }
        if let match = publishers.first(where: { $0.publicKeyHex.lowercased() == key }) {
            return .trusted(match)
        }
        if publishers.isEmpty {
            return .firstTrust
        }
        return .unknownPublisher(key)
    }
}

/// Signature facts remembered on a library row.
public struct LibrarySignatureInfo: Codable, Equatable, Sendable {
    /// False for imports that had no sidecar at all.
    public var signed: Bool
    public var publisherName: String?
    public var publicKeyHex: String?
    public var verifiedAt: Date?

    public init(
        signed: Bool,
        publisherName: String? = nil,
        publicKeyHex: String? = nil,
        verifiedAt: Date? = nil
    ) {
        self.signed = signed
        self.publisherName = publisherName
        self.publicKeyHex = publicKeyHex
        self.verifiedAt = verifiedAt
    }

    /// Short status line under the row title.
    public var statusText: String {
        guard signed else { return "未签名" }
        guard let publisherName, !publisherName.isEmpty else { return "已签名" }
        return "已签名 · \(publisherName)"
    }
}

/// A pending first-use trust prompt.
///
/// The copy (or the patch target) is already on disk when this is created; the
/// UI either trusts the key and continues, or cancels and the copy is removed.
public struct PendingTrustRequest: Identifiable, Equatable, Sendable {
    /// What to resume once the key is trusted.
    public enum Action: Equatable, Sendable {
        case importCopy(
            repoPath: String,
            copyURL: URL,
            sourceName: String,
            sourceBytes: Int64,
            sourceHash: String?,
            sourceIsDirectory: Bool
        )
        case applyPatch(itemID: UUID, patchURL: URL, repoPath: String, sigPath: String)
    }

    public let id: UUID
    public let signerName: String
    public let fingerprint: String
    public let publicKeyHex: String
    public let action: Action

    public init(
        id: UUID = UUID(),
        signerName: String,
        fingerprint: String,
        publicKeyHex: String,
        action: Action
    ) {
        self.id = id
        self.signerName = signerName
        self.fingerprint = fingerprint
        self.publicKeyHex = publicKeyHex
        self.action = action
    }

    /// Alert body shared by both flows.
    public var message: String {
        """
        发布者：\(signerName)
        指纹：\(fingerprint)
        公钥：\(publicKeyHex)

        信任后，来自这个公钥的更新会自动通过签名校验。
        """
    }
}

/// A patch that arrived without a sidecar.
public struct PendingUnsignedUpdate: Identifiable, Equatable, Sendable {
    public let id: UUID
    public let itemID: UUID
    public let patchURL: URL

    public init(id: UUID = UUID(), itemID: UUID, patchURL: URL) {
        self.id = id
        self.itemID = itemID
        self.patchURL = patchURL
    }

    public var message: String {
        """
        这个更新包没有 \(ReleaseSidecar.fileExtension) 签名文件，无法确认来源。
        当前版本只能应用带签名的更新：请向发布者索取 \(patchURL.lastPathComponent).\(ReleaseSidecar.fileExtension) 后再试。
        """
    }
}
