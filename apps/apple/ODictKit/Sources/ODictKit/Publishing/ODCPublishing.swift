import Foundation
import ODCFFI
import ODictCore

/// Swift surface over the signing, trust and release entry points of ODCFFI.
///
/// Every call in this file is synchronous and opens its own repository handle
/// (or none at all, for key generation and verification). Callers that already
/// hold a repository open can safely use these functions: SQLite runs in WAL
/// mode with a busy timeout, so a short-lived second connection is fine.
public enum ODCPublishing {
    /// Generate an Ed25519 keypair. Returns the PKCS#8 PEM and the 64 hex digit
    /// public key.
    public static func generateKeypair() throws -> (pem: String, publicHex: String) {
        var pemPointer: UnsafeMutablePointer<CChar>?
        var hexPointer: UnsafeMutablePointer<CChar>?
        var error: UnsafeMutablePointer<CChar>?
        let status = odc_keypair_generate(&pemPointer, &hexPointer, &error)
        guard status == 0, let pemPointer, let hexPointer else {
            throw OdcError.keyFailed(take(error) ?? "无法生成发布者密钥。")
        }
        defer {
            odc_string_free(pemPointer)
            odc_string_free(hexPointer)
        }
        return (String(cString: pemPointer), String(cString: hexPointer))
    }

    /// Validate a PEM private key and return its public key.
    ///
    /// The C ABI has no "derive public key" call, so this signs a throwaway
    /// file in the temporary directory and reads the key out of the sidecar
    /// JSON. The probe file is always removed.
    public static func publicKeyHex(forPEM pem: String) throws -> String {
        let trimmed = pem.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else {
            throw OdcError.keyFailed("私钥内容为空。")
        }
        let probe = FileManager.default.temporaryDirectory
            .appending(path: "odc-keycheck-\(UUID().uuidString).txt")
        do {
            try Data("odc publisher key check\n".utf8).write(to: probe)
        } catch {
            throw OdcError.keyFailed("无法写入临时文件：\(error.localizedDescription)")
        }
        defer { try? FileManager.default.removeItem(at: probe) }

        var json: UnsafeMutablePointer<CChar>?
        var error: UnsafeMutablePointer<CChar>?
        let status = probe.path.withCString { path in
            trimmed.withCString { key in
                odc_sign_file(path, key, nil, &json, &error)
            }
        }
        guard status == 0 else {
            throw OdcError.keyFailed(take(error) ?? "无法解析私钥。")
        }
        let payload: SignaturePayload = try decode(json, context: "私钥校验结果")
        return payload.publicKey
    }

    /// Trusted publishers of the repository at `repoPath`.
    public static func publishers(repoPath: String) throws -> [Publisher] {
        try withRepo(repoPath) { handle in
            var json: UnsafeMutablePointer<CChar>?
            var error: UnsafeMutablePointer<CChar>?
            let status = odc_publishers_json(handle, &json, &error)
            guard status == 0 else {
                throw OdcError.trustFailed(take(error) ?? "无法读取发布者列表。")
            }
            let payload: PublishersPayload = try decode(json, context: "发布者列表")
            return payload.publishers
        }
    }

    /// Trust a publisher key, optionally naming it. Re-adding a known key is
    /// idempotent and returns the stored record.
    @discardableResult
    public static func trustAdd(repoPath: String, publicKeyHex: String, name: String?) throws -> Publisher {
        let key = try validatedPublicKey(publicKeyHex)
        let trimmedName = name?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        return try withRepo(repoPath) { handle in
            var json: UnsafeMutablePointer<CChar>?
            var error: UnsafeMutablePointer<CChar>?
            let status = key.withCString { keyPointer in
                withOptionalCString(trimmedName.isEmpty ? nil : trimmedName) { namePointer in
                    odc_trust_add(handle, keyPointer, namePointer, &json, &error)
                }
            }
            guard status == 0 else {
                throw OdcError.trustFailed(take(error) ?? "无法信任该公钥。")
            }
            let payload: TrustAddPayload = try decode(json, context: "信任结果")
            return payload.publisher
        }
    }

    /// Forget a trusted key. Returns true when a row was removed.
    @discardableResult
    public static func trustRemove(repoPath: String, publicKeyHex: String) throws -> Bool {
        let key = try validatedPublicKey(publicKeyHex)
        return try withRepo(repoPath) { handle in
            var json: UnsafeMutablePointer<CChar>?
            var error: UnsafeMutablePointer<CChar>?
            let status = key.withCString { keyPointer in
                odc_trust_remove(handle, keyPointer, &json, &error)
            }
            guard status == 0 else {
                throw OdcError.trustFailed(take(error) ?? "无法移除该公钥。")
            }
            let payload: TrustRemovePayload = try decode(json, context: "移除结果")
            return payload.removed
        }
    }

    /// Publish a signed release into `outDir`.
    ///
    /// Existing files are never overwritten: the core refuses the whole run
    /// before writing anything, and the error text explains which file is in
    /// the way.
    public static func publish(
        repoPath: String,
        outDir: String,
        name: String,
        label: String,
        base: String?,
        keyPEM: String,
        signer: String?,
        notes: String?
    ) throws -> PublishReport {
        let trimmedLabel = label.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmedLabel.isEmpty else {
            throw OdcError.publishFailed("版本标签不能为空。")
        }
        let trimmedOutDir = outDir.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmedOutDir.isEmpty else {
            throw OdcError.publishFailed("请选择输出目录。")
        }
        let trimmedName = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmedName.isEmpty else {
            throw OdcError.publishFailed("发布名称不能为空。")
        }
        let trimmedPEM = keyPEM.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmedPEM.isEmpty else {
            throw OdcError.publishFailed("尚未配置发布者密钥。")
        }
        let trimmedBase = base?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        let trimmedSigner = signer?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        let trimmedNotes = notes?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""

        return try withRepo(repoPath) { handle in
            var json: UnsafeMutablePointer<CChar>?
            var error: UnsafeMutablePointer<CChar>?
            let status = trimmedOutDir.withCString { outPointer in
                trimmedName.withCString { namePointer in
                    trimmedLabel.withCString { labelPointer in
                        withOptionalCString(trimmedBase.isEmpty ? nil : trimmedBase) { basePointer in
                            trimmedPEM.withCString { pemPointer in
                                withOptionalCString(trimmedSigner.isEmpty ? nil : trimmedSigner) { signerPointer in
                                    withOptionalCString(trimmedNotes.isEmpty ? nil : trimmedNotes) { notesPointer in
                                        odc_publish(
                                            handle,
                                            outPointer,
                                            namePointer,
                                            labelPointer,
                                            basePointer,
                                            pemPointer,
                                            signerPointer,
                                            notesPointer,
                                            &json,
                                            &error
                                        )
                                    }
                                }
                            }
                        }
                    }
                }
            }
            guard status == 0 else {
                throw OdcError.publishFailed(take(error) ?? "发布失败。")
            }
            return try decode(json, context: "发布结果")
        }
    }

    /// Verify a release directory. Cryptographic failures come back as a
    /// report with valid false instead of an error.
    public static func verifyRelease(dir: String, expectedPublicKeyHex: String? = nil) throws -> ReleaseVerifyReport {
        let trimmedDir = dir.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmedDir.isEmpty else {
            throw OdcError.verifyFailed("请选择要校验的发布目录。")
        }
        var expected: String?
        if let expectedPublicKeyHex, !expectedPublicKeyHex.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            expected = try validatedPublicKey(expectedPublicKeyHex)
        }

        var json: UnsafeMutablePointer<CChar>?
        var error: UnsafeMutablePointer<CChar>?
        let status = trimmedDir.withCString { dirPointer in
            withOptionalCString(expected) { keyPointer in
                odc_verify_release(dirPointer, keyPointer, &json, &error)
            }
        }
        guard status == 0 else {
            throw OdcError.verifyFailed(take(error) ?? "无法校验发布目录。")
        }
        return try decode(json, context: "校验结果")
    }

    /// Sign `path` with a PKCS#8 PEM private key and return the sidecar JSON.
    ///
    /// The app normally publishes through `publish`; this entry point exists so
    /// tests and the release wizard can create a signature for a single file.
    public static func signFile(path: String, keyPEM: String, signer: String? = nil) throws -> String {
        let trimmedPath = path.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmedPath.isEmpty else {
            throw OdcError.keyFailed("请选择要签名的文件。")
        }
        var json: UnsafeMutablePointer<CChar>?
        var error: UnsafeMutablePointer<CChar>?
        let status = trimmedPath.withCString { pathPointer in
            keyPEM.withCString { keyPointer in
                withOptionalCString(signer) { signerPointer in
                    odc_sign_file(pathPointer, keyPointer, signerPointer, &json, &error)
                }
            }
        }
        guard status == 0 else {
            throw OdcError.keyFailed(take(error) ?? "无法签名该文件。")
        }
        guard let sidecar = take(json) else {
            throw OdcError.invalidResponse("签名结果：内核没有返回数据")
        }
        return sidecar
    }

    /// Verify `path` against the sidecar text stored at `sigPath`.
    ///
    /// An invalid signature is a report with `valid == false`, not an error;
    /// only an unreadable sidecar or a broken FFI call throws.
    public static func verifySignatureFile(
        path: String,
        sigPath: String,
        expectedPublicKeyHex: String? = nil
    ) throws -> SignatureVerification {
        let trimmedPath = path.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmedPath.isEmpty else {
            throw OdcError.verifyFailed("请选择要校验的文件。")
        }
        let signatureText: String
        do {
            signatureText = try String(contentsOfFile: sigPath, encoding: .utf8)
                .trimmingCharacters(in: .whitespacesAndNewlines)
        } catch {
            throw OdcError.verifyFailed("无法读取签名文件：\(error.localizedDescription)")
        }
        guard !signatureText.isEmpty else {
            throw OdcError.verifyFailed("签名文件为空。")
        }
        var expected: String?
        if let expectedPublicKeyHex {
            expected = try validatedPublicKey(expectedPublicKeyHex)
        }
        var json: UnsafeMutablePointer<CChar>?
        var error: UnsafeMutablePointer<CChar>?
        let status = trimmedPath.withCString { pathPointer in
            signatureText.withCString { sigPointer in
                withOptionalCString(expected) { keyPointer in
                    odc_verify_signature_file(pathPointer, sigPointer, keyPointer, &json, &error)
                }
            }
        }
        guard status == 0 else {
            throw OdcError.verifyFailed(take(error) ?? "无法校验签名。")
        }
        return try decode(json, context: "签名校验结果")
    }

    /// Verify the signature of `patchPath` and apply it to the repository.
    ///
    /// Verification runs before any mutation inside the core, so a rejected
    /// signature leaves the repository untouched. Conflicts are recorded in the
    /// repository and only counted in the report; they are never resolved here.
    public static func applySignedPatch(
        repoPath: String,
        patchPath: String,
        sigPath: String,
        allowTrustNew: Bool = false
    ) throws -> PatchApplyReport {
        let trimmedPatch = patchPath.trimmingCharacters(in: .whitespacesAndNewlines)
        let trimmedSignature = sigPath.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmedPatch.isEmpty, !trimmedSignature.isEmpty else {
            throw OdcError.patchFailed("请选择更新包及其签名文件。")
        }
        return try withRepo(repoPath) { handle in
            var json: UnsafeMutablePointer<CChar>?
            var error: UnsafeMutablePointer<CChar>?
            let status = trimmedPatch.withCString { patchPointer in
                trimmedSignature.withCString { sigPointer in
                    odc_patch_apply_signed(
                        handle,
                        patchPointer,
                        sigPointer,
                        allowTrustNew ? 1 : 0,
                        &json,
                        &error
                    )
                }
            }
            guard status == 0 else {
                throw OdcError.patchFailed(take(error) ?? "无法应用更新。")
            }
            return try decode(json, context: "更新结果")
        }
    }

    // MARK: - Plumbing

    private struct PublishersPayload: Decodable {
        let publishers: [Publisher]
    }

    private struct TrustAddPayload: Decodable {
        let publisher: Publisher
    }

    private struct TrustRemovePayload: Decodable {
        let removed: Bool
    }

    private struct SignaturePayload: Decodable {
        let publicKey: String

        private enum CodingKeys: String, CodingKey {
            case publicKey = "public_key"
        }
    }

    private static func validatedPublicKey(_ hex: String) throws -> String {
        let normalised = PublisherIdentity.normalizedPublicKeyHex(hex)
        guard PublisherIdentity.isValidPublicKeyHex(normalised) else {
            throw OdcError.trustFailed("公钥必须是 64 位十六进制字符。")
        }
        return normalised
    }

    private static func withRepo<T>(_ path: String, _ body: (OpaquePointer) throws -> T) throws -> T {
        var error: UnsafeMutablePointer<CChar>?
        let opened = path.withCString { odc_repo_open($0, &error) }
        guard let opened else {
            throw OdcError.openFailed(take(error) ?? "无法打开词典 \(path)")
        }
        defer { odc_repo_close(opened) }
        return try body(opened)
    }

    private static func withOptionalCString<T>(
        _ value: String?,
        _ body: (UnsafePointer<CChar>?) throws -> T
    ) rethrows -> T {
        guard let value else { return try body(nil) }
        return try value.withCString { pointer in try body(pointer) }
    }

    private static func decode<T: Decodable>(_ pointer: UnsafeMutablePointer<CChar>?, context: String) throws -> T {
        guard let pointer else {
            throw OdcError.invalidResponse("\(context)：内核没有返回数据")
        }
        defer { odc_string_free(pointer) }
        let data = Data(bytes: pointer, count: strlen(pointer))
        do {
            return try JSONDecoder().decode(T.self, from: data)
        } catch {
            throw OdcError.invalidResponse("\(context)：\(error)")
        }
    }

    private static func take(_ pointer: UnsafeMutablePointer<CChar>?) -> String? {
        guard let pointer else { return nil }
        defer { odc_string_free(pointer) }
        return String(cString: pointer)
    }
}
