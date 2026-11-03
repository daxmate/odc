import Foundation
import XCTest
@testable import ODictKit

/// End-to-end cover for the FFI wrappers against the committed demo repository.
///
/// These calls go through the real C ABI, so they also guard the argument
/// order of odc_trust_add / odc_publish / odc_verify_release.
final class PublishingIntegrationTests: XCTestCase {
    func testPublishingAndVerifyingARelease() throws {
        let container = try Fixture.temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: container) }

        let repo = container.appending(path: "repo", directoryHint: .isDirectory)
        try FileManager.default.copyItem(at: Fixture.demoRepository, to: repo)
        let keypair = try ODCPublishing.generateKeypair()

        let trusted = try ODCPublishing.trustAdd(
            repoPath: repo.path,
            publicKeyHex: keypair.publicHex.uppercased(),
            name: "测试发布者"
        )
        XCTAssertEqual(trusted.name, "测试发布者")
        XCTAssertEqual(trusted.publicKeyHex, keypair.publicHex)
        XCTAssertEqual(trusted.source, .manual)

        let listed = try ODCPublishing.publishers(repoPath: repo.path)
        XCTAssertEqual(listed.count, 1)
        XCTAssertEqual(listed.first?.publicKeyHex, keypair.publicHex)
        XCTAssertNotNil(listed.first?.addedAt)

        let outDir = container.appending(path: "dist", directoryHint: .isDirectory)
        try FileManager.default.createDirectory(at: outDir, withIntermediateDirectories: true)
        let report = try ODCPublishing.publish(
            repoPath: repo.path,
            outDir: outDir.path,
            name: "demo",
            label: "2026.11.03",
            base: nil,
            keyPEM: keypair.pem,
            signer: "tester",
            notes: "integration"
        )
        XCTAssertFalse(report.head.isEmpty)
        XCTAssertFalse(report.artifacts.isEmpty)
        XCTAssertEqual(report.publisherPublicKey, keypair.publicHex)
        XCTAssertNil(report.base)
        XCTAssertTrue(FileManager.default.fileExists(atPath: report.outDir))

        let verified = try ODCPublishing.verifyRelease(dir: outDir.path, expectedPublicKeyHex: keypair.publicHex)
        XCTAssertTrue(verified.valid, verified.failureDescription ?? "")
        XCTAssertEqual(verified.label, "2026.11.03")
        XCTAssertEqual(verified.head, report.head)
        XCTAssertEqual(verified.publisherPublicKey, keypair.publicHex)

        let pinnedToAnotherKey = try ODCPublishing.generateKeypair().publicHex
        let rejected = try ODCPublishing.verifyRelease(dir: outDir.path, expectedPublicKeyHex: pinnedToAnotherKey)
        XCTAssertFalse(rejected.valid)
        XCTAssertNotNil(rejected.failureDescription)

        XCTAssertTrue(try ODCPublishing.trustRemove(repoPath: repo.path, publicKeyHex: keypair.publicHex))
        XCTAssertTrue(try ODCPublishing.publishers(repoPath: repo.path).isEmpty)
    }
}
