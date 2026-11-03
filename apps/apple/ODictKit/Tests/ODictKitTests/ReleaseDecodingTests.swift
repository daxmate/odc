import Foundation
import XCTest
@testable import ODictKit

final class ReleaseDecodingTests: XCTestCase {
    private struct PublishersPayload: Decodable {
        let publishers: [Publisher]
    }

    private func decode<T: Decodable>(_ type: T.Type, _ json: String) throws -> T {
        try JSONDecoder().decode(type, from: Data(json.utf8))
    }

    func testPublishersPayloadDecodesTrustRows() throws {
        let json = """
        {"publishers": [
            {"public_key": "AB12", "name": "alice", "added_at": 1700000000, "source": "manual"},
            {"public_key": "CD34", "name": "", "added_at": 1700000100, "source": "tofu"},
            {"public_key": "EF56", "name": "eve", "added_at": 1700000200, "source": "web"}
        ]}
        """

        let payload = try decode(PublishersPayload.self, json)

        XCTAssertEqual(payload.publishers.count, 3)
        XCTAssertEqual(payload.publishers[0].publicKeyHex, "AB12")
        XCTAssertEqual(payload.publishers[0].name, "alice")
        XCTAssertEqual(payload.publishers[0].addedAt, Date(timeIntervalSince1970: 1_700_000_000))
        XCTAssertEqual(payload.publishers[0].source, .manual)
        XCTAssertEqual(payload.publishers[1].source, .tofu)
        XCTAssertEqual(payload.publishers[1].displayName.isEmpty, false)
        XCTAssertEqual(payload.publishers[2].source, .unknown("web"))
    }

    func testPublishReportDecodesArtifactsAndPublisher() throws {
        let json = """
        {
            "out_dir": "/tmp/dist",
            "manifest": "/tmp/dist/manifest.json",
            "head": "abcdef0123456789",
            "base": null,
            "artifacts": [
                {"kind": "pack", "path": "demo.odc", "size": 1234, "blake3": "ff00"},
                {"kind": "manifest", "path": "manifest.json", "size": 99, "blake3": "ee00"}
            ],
            "publisher": {"public_key": "\(PublishingFixtures.publicKeyA)"}
        }
        """

        let report = try decode(PublishReport.self, json)

        XCTAssertEqual(report.outDir, "/tmp/dist")
        XCTAssertEqual(report.manifestPath, "/tmp/dist/manifest.json")
        XCTAssertEqual(report.head, "abcdef0123456789")
        XCTAssertNil(report.base)
        XCTAssertEqual(report.artifacts.count, 2)
        XCTAssertEqual(report.artifacts[0].fileName, "demo.odc")
        XCTAssertEqual(report.artifacts[0].size, 1234)
        XCTAssertEqual(report.publisherPublicKey, PublishingFixtures.publicKeyA)
        XCTAssertFalse(report.publisherFingerprint.isEmpty)
    }

    func testVerifyReportWithValidReleaseHasNoFailureText() throws {
        let json = """
        {
            "valid": true,
            "dir": "/tmp/dist",
            "name": "demo",
            "label": "2026.11.03",
            "head": "abcdef0123456789",
            "base": "0123456789abcdef",
            "created_at": "2026-11-03T12:00:00Z",
            "publisher": {"public_key": "\(PublishingFixtures.publicKeyA)", "name": "alice"},
            "artifacts": [{"kind": "pack", "path": "demo.odc", "size": 12, "blake3": "aa"}]
        }
        """

        let report = try decode(ReleaseVerifyReport.self, json)

        XCTAssertTrue(report.valid)
        XCTAssertEqual(report.label, "2026.11.03")
        XCTAssertEqual(report.base, "0123456789abcdef")
        XCTAssertEqual(report.createdAt, "2026-11-03T12:00:00Z")
        XCTAssertEqual(report.publisherName, "alice")
        XCTAssertEqual(report.publisherPublicKey, PublishingFixtures.publicKeyA)
        XCTAssertEqual(report.artifacts.count, 1)
        XCTAssertNil(report.failureDescription)
    }

    func testVerifyReportWithCryptographicFailureKeepsCodes() throws {
        let json = """
        {"valid": false, "error_code": "ODC-E034", "error": "签名不匹配"}
        """

        let report = try decode(ReleaseVerifyReport.self, json)

        XCTAssertFalse(report.valid)
        XCTAssertEqual(report.errorCode, "ODC-E034")
        XCTAssertEqual(report.failureDescription, "ODC-E034 签名不匹配")
    }

    func testVerifyReportAcceptsNumericCreationTime() throws {
        let json = """
        {"valid": true, "created_at": 1700000000}
        """

        let report = try decode(ReleaseVerifyReport.self, json)

        XCTAssertEqual(report.createdAt, "1700000000")
    }
}
