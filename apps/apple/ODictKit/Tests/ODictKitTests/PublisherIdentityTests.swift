import Foundation
import XCTest
@testable import ODictKit

final class PublisherIdentityTests: XCTestCase {
    func testFingerprintHasEightGroupsOfFourUppercaseHexDigits() {
        let identity = PublisherIdentity(name: "alice", publicKeyHex: PublishingFixtures.publicKeyA)
        let groups = identity.fingerprint.split(separator: " ")

        XCTAssertEqual(groups.count, 8)
        for group in groups {
            XCTAssertEqual(group.count, 4)
            XCTAssertTrue(group.allSatisfy(\.isHexDigit))
            XCTAssertEqual(String(group), group.uppercased())
        }
    }

    func testFingerprintIsStableAndKeySpecific() {
        let first = PublisherIdentity(name: "alice", publicKeyHex: PublishingFixtures.publicKeyA)
        let same = PublisherIdentity(name: "renamed", publicKeyHex: PublishingFixtures.publicKeyA.uppercased())
        let other = PublisherIdentity(name: "bob", publicKeyHex: PublishingFixtures.publicKeyB)

        XCTAssertEqual(first.fingerprint, same.fingerprint)
        XCTAssertNotEqual(first.fingerprint, other.fingerprint)
    }

    func testShortFingerprintUsesTheFirstTwoGroups() {
        let identity = PublisherIdentity(name: "alice", publicKeyHex: PublishingFixtures.publicKeyA)
        let expected = identity.fingerprint.split(separator: " ").prefix(2).joined(separator: " ")

        XCTAssertEqual(identity.shortFingerprint, expected)
        XCTAssertEqual(identity.shortFingerprint.split(separator: " ").count, 2)
    }

    func testPublicKeyValidationAcceptsOnly64HexDigits() {
        XCTAssertTrue(PublisherIdentity.isValidPublicKeyHex(PublishingFixtures.publicKeyA))
        XCTAssertTrue(PublisherIdentity.isValidPublicKeyHex(PublishingFixtures.publicKeyA.uppercased()))
        XCTAssertFalse(PublisherIdentity.isValidPublicKeyHex("abc"))
        XCTAssertFalse(PublisherIdentity.isValidPublicKeyHex(String(repeating: "zz", count: 32)))
    }

    func testNormalisationTrimsAndLowercases() {
        let raw = "  \(PublishingFixtures.publicKeyA.uppercased())\n"

        XCTAssertEqual(PublisherIdentity.normalizedPublicKeyHex(raw), PublishingFixtures.publicKeyA)
    }

    func testCodableRoundTripCarriesNoPrivateKey() throws {
        let identity = PublisherIdentity(
            name: "alice",
            publicKeyHex: PublishingFixtures.publicKeyA,
            createdAt: Date(timeIntervalSince1970: 1_700_000_000)
        )

        let data = try JSONEncoder().encode(identity)
        let text = try XCTUnwrap(String(data: data, encoding: .utf8))
        XCTAssertFalse(text.contains("PRIVATE"))
        XCTAssertEqual(try JSONDecoder().decode(PublisherIdentity.self, from: data), identity)
    }
}
