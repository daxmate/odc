import Foundation
import XCTest

@testable import ODictKit

@MainActor
final class ReleaseImportTests: XCTestCase {
    // MARK: - Sidecar paths

    func testSignatureSidecarAppendsSigToTheArtifactName() {
        let artifact = URL(fileURLWithPath: "/tmp/demo.odc")
        XCTAssertEqual(
            ReleaseSidecar.signatureURL(for: artifact).path,
            "/tmp/demo.odc.sig"
        )
    }

    func testHasSignatureIsFalseWhenTheSidecarIsMissing() {
        let directory = try! Fixture.temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let artifact = directory.appending(path: "demo.odc")
        try! Data("x".utf8).write(to: artifact)

        XCTAssertFalse(ReleaseSidecar.hasSignature(for: artifact))
        try! Data("{}".utf8).write(to: ReleaseSidecar.signatureURL(for: artifact))
        XCTAssertTrue(ReleaseSidecar.hasSignature(for: artifact))
    }

    // MARK: - Decoding

    func testValidSignatureDecodesEveryField() throws {
        let json = """
        {
          "valid": true,
          "public_key": "abababababababababababababababababababababababababababababababab",
          "created_at": "2026-11-03T12:00:00Z",
          "signer": "alice",
          "hash": "0123456789abcdef",
          "size": 42
        }
        """
        let verification = try JSONDecoder().decode(
            SignatureVerification.self,
            from: Data(json.utf8)
        )
        XCTAssertTrue(verification.valid)
        XCTAssertEqual(verification.signer, "alice")
        XCTAssertEqual(verification.displayName, "alice")
        XCTAssertEqual(verification.createdAt, "2026-11-03T12:00:00Z")
        XCTAssertEqual(verification.size, 42)
        XCTAssertEqual(verification.errorCode, nil)
        XCTAssertNotNil(verification.fingerprint)
    }

    func testInvalidSignatureDecodesTheErrorCode() throws {
        let json = """
        {"valid": false, "error_code": "ODC-E033", "error": "signature mismatch"}
        """
        let verification = try JSONDecoder().decode(
            SignatureVerification.self,
            from: Data(json.utf8)
        )
        XCTAssertFalse(verification.valid)
        XCTAssertEqual(verification.failureText, "ODC-E033 signature mismatch")
    }

    func testMissingSignerFallsBackToUnnamedPublisher() throws {
        let json = """
        {"valid": true, "public_key": "\(PublishingFixtures.publicKeyA)", "signer": "  "}
        """
        let verification = try JSONDecoder().decode(
            SignatureVerification.self,
            from: Data(json.utf8)
        )
        XCTAssertEqual(verification.displayName, "未命名发布者")
    }

    func testAppliedPatchReportDecodesCounts() throws {
        let json = """
        {
          "applied": true,
          "merged": false,
          "target": "abc",
          "base_blocks": 1,
          "new_blocks": 2,
          "index_changes": 3,
          "bytes": 99
        }
        """
        let report = try JSONDecoder().decode(PatchApplyReport.self, from: Data(json.utf8))
        XCTAssertTrue(report.applied)
        XCTAssertFalse(report.merged)
        XCTAssertEqual(report.newBlocks, 2)
        XCTAssertEqual(report.conflicts, 0)
        XCTAssertEqual(report.summaryText, "已应用更新，新增块 2，索引变更 3。")
    }

    func testMergedPatchReportCarriesTheConflictCount() throws {
        let json = """
        {"applied": true, "merged": true, "conflicts": 2, "new_blocks": 1}
        """
        let report = try JSONDecoder().decode(PatchApplyReport.self, from: Data(json.utf8))
        XCTAssertTrue(report.merged)
        XCTAssertEqual(report.conflicts, 2)
        XCTAssertTrue(report.summaryText.contains("2 处冲突"))
    }

    // MARK: - Trust decision matrix

    private func verification(
        valid: Bool = true,
        key: String? = PublishingFixtures.publicKeyA
    ) -> SignatureVerification {
        SignatureVerification(valid: valid, publicKeyHex: key, signer: "alice")
    }

    private func publisher(_ key: String) -> Publisher {
        Publisher(publicKeyHex: key, name: "alice", addedAt: nil, source: .manual)
    }

    func testInvalidSignatureIsRejectedBeforeTheTrustStore() {
        let decision = TrustDecision.evaluate(
            verification: verification(valid: false),
            publishers: []
        )
        XCTAssertEqual(decision, .invalidSignature("签名无效。"))
    }

    func testMissingKeyIsRejected() {
        let decision = TrustDecision.evaluate(
            verification: verification(key: nil),
            publishers: []
        )
        XCTAssertEqual(decision, .missingPublicKey)
    }

    func testValidSignatureWithEmptyStoreAsksForFirstTrust() {
        let decision = TrustDecision.evaluate(verification: verification(), publishers: [])
        XCTAssertEqual(decision, .firstTrust)
    }

    func testKnownKeyIsTrusted() {
        let decision = TrustDecision.evaluate(
            verification: verification(),
            publishers: [publisher(PublishingFixtures.publicKeyA)]
        )
        guard case .trusted(let stored) = decision else {
            return XCTFail("expected trusted, got \(decision)")
        }
        XCTAssertEqual(stored.publicKeyHex, PublishingFixtures.publicKeyA)
    }

    func testUnknownKeyAgainstANonEmptyStoreIsRefused() {
        let decision = TrustDecision.evaluate(
            verification: verification(),
            publishers: [publisher(PublishingFixtures.publicKeyB)]
        )
        guard case .unknownPublisher(let key) = decision else {
            return XCTFail("expected unknownPublisher, got \(decision)")
        }
        XCTAssertEqual(key, PublishingFixtures.publicKeyA)
    }

    func testStatusTextDistinguishesSignedAndUnsignedRows() {
        XCTAssertEqual(
            LibrarySignatureInfo(signed: true, publisherName: "alice").statusText,
            "已签名 · alice"
        )
        XCTAssertEqual(LibrarySignatureInfo(signed: true).statusText, "已签名")
        XCTAssertEqual(LibrarySignatureInfo(signed: false).statusText, "未签名")
    }

    // MARK: - Real signatures through the FFI

    func testSignAndVerifyRoundTrip() throws {
        let directory = try Fixture.temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let artifact = directory.appending(path: "demo.bin")
        try Data("odc signature round trip".utf8).write(to: artifact)

        let keypair = try ODCPublishing.generateKeypair()
        let sidecar = try ODCPublishing.signFile(
            path: artifact.path,
            keyPEM: keypair.pem,
            signer: "alice"
        )
        let sigURL = ReleaseSidecar.signatureURL(for: artifact)
        try Data(sidecar.utf8).write(to: sigURL)

        let verification = try ODCPublishing.verifySignatureFile(
            path: artifact.path,
            sigPath: sigURL.path
        )
        XCTAssertTrue(verification.valid)
        XCTAssertEqual(verification.signer, "alice")
        XCTAssertEqual(
            verification.publicKeyHex?.lowercased(),
            keypair.publicHex.lowercased()
        )
    }

    func testTamperedFileFailsVerificationWithoutThrowing() throws {
        let directory = try Fixture.temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let artifact = directory.appending(path: "demo.bin")
        try Data("original".utf8).write(to: artifact)

        let keypair = try ODCPublishing.generateKeypair()
        let sidecar = try ODCPublishing.signFile(path: artifact.path, keyPEM: keypair.pem)
        let sigURL = ReleaseSidecar.signatureURL(for: artifact)
        try Data(sidecar.utf8).write(to: sigURL)

        try Data("tampered".utf8).write(to: artifact)
        let verification = try ODCPublishing.verifySignatureFile(
            path: artifact.path,
            sigPath: sigURL.path
        )
        XCTAssertFalse(verification.valid)
        XCTAssertNotNil(verification.errorCode)
    }

    func testPinnedKeyMismatchIsAnInvalidReport() throws {
        let directory = try Fixture.temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let artifact = directory.appending(path: "demo.bin")
        try Data("pinned".utf8).write(to: artifact)

        let keypair = try ODCPublishing.generateKeypair()
        let sidecar = try ODCPublishing.signFile(path: artifact.path, keyPEM: keypair.pem)
        let sigURL = ReleaseSidecar.signatureURL(for: artifact)
        try Data(sidecar.utf8).write(to: sigURL)

        let verification = try ODCPublishing.verifySignatureFile(
            path: artifact.path,
            sigPath: sigURL.path,
            expectedPublicKeyHex: PublishingFixtures.publicKeyB
        )
        XCTAssertFalse(verification.valid)
    }

    func testBadPatchFailsWithoutCorruptingTheRepository() throws {
        let directory = try Fixture.temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let repo = directory.appending(path: "repo", directoryHint: .isDirectory)
        try FileManager.default.copyItem(at: Fixture.demoRepository, to: repo)
        let patch = directory.appending(path: "broken.odp")
        try Data("not a patch".utf8).write(to: patch)
        let signature = directory.appending(path: "broken.odp.sig")
        try Data("{}".utf8).write(to: signature)

        XCTAssertThrowsError(
            try ODCPublishing.applySignedPatch(
                repoPath: repo.path,
                patchPath: patch.path,
                sigPath: signature.path
            )
        )
        // The failure happened before any write: the repository still opens.
        let reopened = try OdcDictionary(url: repo)
        reopened.close()
    }

    // MARK: - Version consistency

    func testVersionIsConsistentAcrossCargoProjectAndInfoPlists() throws {
        let root = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()

        let cargo = try String(
            contentsOf: root.appending(path: "Cargo.toml"),
            encoding: .utf8
        )
        XCTAssertTrue(
            cargo.contains("version = \"0.14.0\""),
            "Cargo.toml workspace version is not 0.14.0"
        )

        let project = try String(
            contentsOf: root.appending(path: "apps/apple/project.yml"),
            encoding: .utf8
        )
        XCTAssertEqual(
            project.components(separatedBy: "CFBundleShortVersionString: \"0.14.0\"").count - 1,
            2,
            "both targets must declare 0.14.0"
        )
        XCTAssertEqual(
            project.components(separatedBy: "CFBundleVersion: \"16\"").count - 1,
            2,
            "both targets must declare build 16"
        )

        for name in ["ODict", "ODictDesktop"] {
            let plistURL = root.appending(path: "apps/apple/\(name)/Info.plist")
            let data = try Data(contentsOf: plistURL)
            let plist = try PropertyListSerialization.propertyList(
                from: data,
                options: [],
                format: nil
            )
            let dictionary = try XCTUnwrap(plist as? [String: Any])
            XCTAssertEqual(dictionary["CFBundleShortVersionString"] as? String, "0.14.0")
            XCTAssertEqual(dictionary["CFBundleVersion"] as? String, "16")
        }
    }
}
