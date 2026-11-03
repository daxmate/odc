import Foundation
import XCTest
@testable import ODictKit

final class PublishingValidationTests: XCTestCase {
    func testPublishRejectsAnEmptyLabelBeforeOpeningTheRepo() {
        XCTAssertThrowsError(
            try ODCPublishing.publish(
                repoPath: "/tmp/odc-missing-repo",
                outDir: "/tmp/odc-missing-out",
                name: "demo",
                label: "   ",
                base: nil,
                keyPEM: "-----BEGIN PRIVATE KEY-----",
                signer: nil,
                notes: nil
            )
        ) { error in
            guard case OdcError.publishFailed(let message) = error else {
                return XCTFail("unexpected error: \(error)")
            }
            XCTAssertTrue(message.contains("版本标签"))
        }
    }

    func testPublishRejectsAMissingOutputDirectory() {
        XCTAssertThrowsError(
            try ODCPublishing.publish(
                repoPath: "/tmp/odc-missing-repo",
                outDir: "  ",
                name: "demo",
                label: "2026.11.03",
                base: nil,
                keyPEM: "pem",
                signer: nil,
                notes: nil
            )
        ) { error in
            guard case OdcError.publishFailed(let message) = error else {
                return XCTFail("unexpected error: \(error)")
            }
            XCTAssertTrue(message.contains("输出目录"))
        }
    }

    func testPublishRejectsAMissingKeyBeforeOpeningTheRepo() {
        XCTAssertThrowsError(
            try ODCPublishing.publish(
                repoPath: "/tmp/odc-missing-repo",
                outDir: "/tmp/odc-missing-out",
                name: "demo",
                label: "2026.11.03",
                base: nil,
                keyPEM: "",
                signer: nil,
                notes: nil
            )
        ) { error in
            guard case OdcError.publishFailed(let message) = error else {
                return XCTFail("unexpected error: \(error)")
            }
            XCTAssertTrue(message.contains("密钥"))
        }
    }

    func testTrustAddRejectsAMalformedKey() {
        XCTAssertThrowsError(
            try ODCPublishing.trustAdd(repoPath: "/tmp/odc-missing-repo", publicKeyHex: "ABCD", name: nil)
        ) { error in
            guard case OdcError.trustFailed(let message) = error else {
                return XCTFail("unexpected error: \(error)")
            }
            XCTAssertTrue(message.contains("64"))
        }
    }

    func testVerifyReleaseRejectsAnEmptyDirectory() {
        XCTAssertThrowsError(try ODCPublishing.verifyRelease(dir: "  ")) { error in
            guard case OdcError.verifyFailed = error else {
                return XCTFail("unexpected error: \(error)")
            }
        }
    }

    func testOpeningAMissingRepoReportsThePath() {
        XCTAssertThrowsError(try ODCPublishing.publishers(repoPath: "/tmp/odc-definitely-missing")) { error in
            guard case OdcError.openFailed(let message) = error else {
                return XCTFail("unexpected error: \(error)")
            }
            XCTAssertFalse(message.isEmpty)
        }
    }

    func testGeneratedKeypairIsUsableAndSelfConsistent() throws {
        let keypair = try ODCPublishing.generateKeypair()

        XCTAssertEqual(keypair.publicHex.count, 64)
        XCTAssertTrue(PublisherIdentity.isValidPublicKeyHex(keypair.publicHex))
        XCTAssertEqual(keypair.publicHex, keypair.publicHex.lowercased())
        XCTAssertTrue(keypair.pem.contains("PRIVATE KEY"))
        XCTAssertEqual(try ODCPublishing.publicKeyHex(forPEM: keypair.pem), keypair.publicHex)
    }

    func testImportingABlankPEMFails() {
        XCTAssertThrowsError(try ODCPublishing.publicKeyHex(forPEM: "   ")) { error in
            guard case OdcError.keyFailed(let message) = error else {
                return XCTFail("unexpected error: \(error)")
            }
            XCTAssertTrue(message.contains("为空"))
        }
    }
}
