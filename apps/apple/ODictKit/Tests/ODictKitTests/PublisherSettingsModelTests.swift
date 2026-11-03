import Foundation
import XCTest
@testable import ODictKit

@MainActor
final class PublisherSettingsModelTests: XCTestCase {
    func testGeneratingAKeyStoresIdentityAndPEM() {
        let store = InMemoryPublisherKeyStore()
        let model = PublisherSettingsModel(keyStore: store, fallbackName: "测试机器")
        model.name = "大象"

        model.generate()

        XCTAssertNil(model.errorMessage)
        let identity = model.identity
        XCTAssertEqual(identity?.name, "大象")
        XCTAssertEqual(identity?.publicKeyHex.count, 64)
        XCTAssertTrue(PublisherIdentity.isValidPublicKeyHex(identity?.publicKeyHex ?? ""))
        XCTAssertTrue(store.savedPEM?.contains("PRIVATE KEY") ?? false)
        XCTAssertEqual(store.identity, identity)
    }

    func testImportingAGeneratedKeyKeepsItsPublicKey() throws {
        let keypair = try ODCPublishing.generateKeypair()
        let store = InMemoryPublisherKeyStore()
        let model = PublisherSettingsModel(keyStore: store, fallbackName: "测试机器")
        model.name = "导入测试"

        model.importKey(pem: keypair.pem)

        XCTAssertNil(model.errorMessage)
        XCTAssertEqual(model.identity?.publicKeyHex, keypair.publicHex.lowercased())
        XCTAssertEqual(store.savedPEM, keypair.pem)
    }

    func testImportingGarbageKeepsThePreviousIdentity() {
        let existing = PublisherIdentity(name: "alice", publicKeyHex: PublishingFixtures.publicKeyA)
        let store = InMemoryPublisherKeyStore(identity: existing, pem: "-----BEGIN PRIVATE KEY-----\nnot real\n")
        let model = PublisherSettingsModel(keyStore: store, fallbackName: "测试机器")

        model.importKey(pem: "definitely not a pem")

        XCTAssertNotNil(model.errorMessage)
        XCTAssertEqual(model.identity, existing)
        XCTAssertEqual(store.savedPEM, "-----BEGIN PRIVATE KEY-----\nnot real\n")
    }

    func testDeletingClearsTheIdentity() {
        let existing = PublisherIdentity(name: "alice", publicKeyHex: PublishingFixtures.publicKeyA)
        let store = InMemoryPublisherKeyStore(identity: existing, pem: "pem")
        let model = PublisherSettingsModel(keyStore: store, fallbackName: "测试机器")

        model.delete()

        XCTAssertNil(model.errorMessage)
        XCTAssertNil(model.identity)
        XCTAssertFalse(model.hasIdentity)
        XCTAssertEqual(store.deleteCount, 1)
    }

    func testBlankNameFallsBackToTheMachineName() {
        let store = InMemoryPublisherKeyStore()
        let model = PublisherSettingsModel(keyStore: store, fallbackName: "测试机器")
        model.name = "   "

        model.generate()

        XCTAssertEqual(model.identity?.name, "测试机器")
    }
}
