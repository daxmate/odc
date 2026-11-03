import Foundation
import ODCFFI

/// How a publisher key ended up in a repository's trust store.
public enum PublisherSource: Equatable, Sendable {
    /// The user added the key by hand.
    case manual
    /// Recorded automatically on first use.
    case tofu
    /// A source this build does not know yet; kept verbatim for display.
    case unknown(String)

    /// Text the UI can show without switching on the case.
    public var displayName: String {
        switch self {
        case .manual: return "手动添加"
        case .tofu: return "首次使用自动信任"
        case .unknown(let raw): return raw.isEmpty ? "未知来源" : raw
        }
    }
}

extension PublisherSource: Codable {
    public init(from decoder: Decoder) throws {
        let container = try decoder.singleValueContainer()
        let raw = try container.decode(String.self)
        switch raw {
        case "manual": self = .manual
        case "tofu": self = .tofu
        default: self = .unknown(raw)
        }
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.singleValueContainer()
        switch self {
        case .manual: try container.encode("manual")
        case .tofu: try container.encode("tofu")
        case .unknown(let raw): try container.encode(raw)
        }
    }
}

/// One trusted publisher row (odc_publishers_json / odc_trust_add).
public struct Publisher: Decodable, Equatable, Sendable, Identifiable {
    /// Lower-case hex public key.
    public let publicKeyHex: String
    /// Optional label; empty when the key was trusted without a name.
    public let name: String
    /// When the repository started trusting the key.
    public let addedAt: Date?
    /// manual or tofu.
    public let source: PublisherSource

    public var id: String { publicKeyHex }

    public init(publicKeyHex: String, name: String, addedAt: Date?, source: PublisherSource) {
        self.publicKeyHex = publicKeyHex
        self.name = name
        self.addedAt = addedAt
        self.source = source
    }

    /// Stable display fingerprint, matching PublisherIdentity.
    public var fingerprint: String {
        PublisherIdentity(name: name, publicKeyHex: publicKeyHex, createdAt: addedAt ?? .distantPast)
            .fingerprint
    }

    /// Name plus fingerprint, or the fingerprint alone for unnamed keys.
    public var displayName: String {
        name.isEmpty ? fingerprint : name
    }

    private enum CodingKeys: String, CodingKey {
        case publicKeyHex = "public_key"
        case name
        case addedAt = "added_at"
        case source
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        publicKeyHex = try container.decode(String.self, forKey: .publicKeyHex)
        name = try container.decodeIfPresent(String.self, forKey: .name) ?? ""
        addedAt = try container.decodeIfPresent(Int64.self, forKey: .addedAt)
            .map { Date(timeIntervalSince1970: TimeInterval($0)) }
        source = try container.decodeIfPresent(PublisherSource.self, forKey: .source) ?? .unknown("")
    }
}

/// One file inside a release directory.
public struct ReleaseArtifact: Decodable, Equatable, Sendable, Identifiable {
    /// pack, patch, manifest or signature.
    public let kind: String
    /// Path relative to the release directory.
    public let path: String
    /// Size in bytes.
    public let size: Int64
    /// Lower-case BLAKE3 digest of the file.
    public let blake3: String

    public var id: String { path }

    /// Last path component, for list rows.
    public var fileName: String { (path as NSString).lastPathComponent }

    public init(kind: String, path: String, size: Int64, blake3: String) {
        self.kind = kind
        self.path = path
        self.size = size
        self.blake3 = blake3
    }
}

/// Result of odc_publish.
public struct PublishReport: Decodable, Equatable, Sendable {
    /// Directory the artifacts were written to.
    public let outDir: String
    /// Path of the signed manifest, when the core wrote one.
    public let manifestPath: String?
    /// Commit the release was built from.
    public let head: String
    /// Base commit of the incremental patch, when one was requested.
    public let base: String?
    /// Every artifact in the release.
    public let artifacts: [ReleaseArtifact]
    /// Public key that signed the manifest and sidecars.
    public let publisherPublicKey: String

    public init(
        outDir: String,
        manifestPath: String?,
        head: String,
        base: String?,
        artifacts: [ReleaseArtifact],
        publisherPublicKey: String
    ) {
        self.outDir = outDir
        self.manifestPath = manifestPath
        self.head = head
        self.base = base
        self.artifacts = artifacts
        self.publisherPublicKey = publisherPublicKey
    }

    /// Fingerprint of the signing key, for the success screen.
    public var publisherFingerprint: String {
        PublisherIdentity(name: "", publicKeyHex: publisherPublicKey, createdAt: .distantPast)
            .fingerprint
    }

    private struct PublisherRef: Decodable {
        let publicKey: String?

        private enum CodingKeys: String, CodingKey {
            case publicKey = "public_key"
        }
    }

    private enum CodingKeys: String, CodingKey {
        case outDir = "out_dir"
        case manifestPath = "manifest"
        case head
        case base
        case artifacts
        case publisher
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        outDir = try container.decode(String.self, forKey: .outDir)
        manifestPath = try container.decodeIfPresent(String.self, forKey: .manifestPath)
        head = try container.decode(String.self, forKey: .head)
        base = try container.decodeIfPresent(String.self, forKey: .base)
        artifacts = try container.decodeIfPresent([ReleaseArtifact].self, forKey: .artifacts) ?? []
        let publisher = try container.decodeIfPresent(PublisherRef.self, forKey: .publisher)
        publisherPublicKey = publisher?.publicKey ?? ""
    }
}

/// Result of odc_verify_release.
///
/// Cryptographic failures are business results: the core returns valid false
/// with an error code, and this type carries it instead of throwing.
public struct ReleaseVerifyReport: Decodable, Equatable, Sendable {
    /// True when every signature and digest checked out.
    public let valid: Bool
    /// Release directory that was verified.
    public let directory: String?
    /// Dictionary name recorded in the manifest.
    public let name: String?
    /// Version label recorded in the manifest.
    public let label: String?
    /// Commit the release was built from.
    public let head: String?
    /// Base commit of the incremental patch, when present.
    public let base: String?
    /// Manifest creation time, as the core recorded it.
    public let createdAt: String?
    /// Name of the publisher in the manifest, when recorded.
    public let publisherName: String?
    /// Public key that signed the manifest.
    public let publisherPublicKey: String?
    /// Artifacts found in the directory.
    public let artifacts: [ReleaseArtifact]
    /// ODC-E0xx code when valid is false.
    public let errorCode: String?
    /// Human-readable failure text when valid is false.
    public let error: String?

    public init(
        valid: Bool,
        directory: String? = nil,
        name: String? = nil,
        label: String? = nil,
        head: String? = nil,
        base: String? = nil,
        createdAt: String? = nil,
        publisherName: String? = nil,
        publisherPublicKey: String? = nil,
        artifacts: [ReleaseArtifact] = [],
        errorCode: String? = nil,
        error: String? = nil
    ) {
        self.valid = valid
        self.directory = directory
        self.name = name
        self.label = label
        self.head = head
        self.base = base
        self.createdAt = createdAt
        self.publisherName = publisherName
        self.publisherPublicKey = publisherPublicKey
        self.artifacts = artifacts
        self.errorCode = errorCode
        self.error = error
    }

    /// "ODC-E034 签名不匹配" style text for the UI, or nil when valid.
    public var failureDescription: String? {
        guard !valid else { return nil }
        switch (errorCode, error) {
        case let (code?, message?): return "\(code) \(message)"
        case let (code?, nil): return code
        case let (nil, message?): return message
        case (nil, nil): return "发布包校验失败。"
        }
    }

    private struct FlexibleText: Decodable {
        let value: String?

        init(from decoder: Decoder) throws {
            let container = try decoder.singleValueContainer()
            if let text = try? container.decode(String.self) {
                value = text
            } else if let seconds = try? container.decode(Int64.self) {
                value = String(seconds)
            } else {
                value = nil
            }
        }
    }

    private struct PublisherRef: Decodable {
        let publicKey: String?
        let name: String?

        private enum CodingKeys: String, CodingKey {
            case publicKey = "public_key"
            case name
        }
    }

    private enum CodingKeys: String, CodingKey {
        case valid
        case directory = "dir"
        case name
        case label
        case head
        case base
        case createdAt = "created_at"
        case publisher
        case artifacts
        case errorCode = "error_code"
        case error
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        valid = try container.decode(Bool.self, forKey: .valid)
        directory = try container.decodeIfPresent(String.self, forKey: .directory)
        name = try container.decodeIfPresent(String.self, forKey: .name)
        label = try container.decodeIfPresent(String.self, forKey: .label)
        head = try container.decodeIfPresent(String.self, forKey: .head)
        base = try container.decodeIfPresent(String.self, forKey: .base)
        createdAt = try container.decodeIfPresent(FlexibleText.self, forKey: .createdAt)?.value
        let publisher = try container.decodeIfPresent(PublisherRef.self, forKey: .publisher)
        publisherName = publisher?.name
        publisherPublicKey = publisher?.publicKey
        artifacts = try container.decodeIfPresent([ReleaseArtifact].self, forKey: .artifacts) ?? []
        errorCode = try container.decodeIfPresent(String.self, forKey: .errorCode)
        error = try container.decodeIfPresent(String.self, forKey: .error)
    }
}
