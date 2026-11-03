import Foundation
@testable import ODictKit

/// Publisher key store that keeps everything in memory.
///
/// The real store talks to the keychain, which CI cannot rely on; this double
/// covers the generate / import / delete paths instead.
@MainActor
final class InMemoryPublisherKeyStore: PublisherKeyStore {
    private(set) var savedPEM: String?
    private(set) var identity: PublisherIdentity?
    private(set) var deleteCount = 0

    init(identity: PublisherIdentity? = nil, pem: String? = nil) {
        self.identity = identity
        self.savedPEM = pem
    }

    func load() throws -> PublisherIdentity? { identity }

    func save(pem: String, identity: PublisherIdentity) throws {
        savedPEM = pem
        self.identity = identity
    }

    func privateKeyPEM() throws -> String? { savedPEM }

    func delete() throws {
        savedPEM = nil
        identity = nil
        deleteCount += 1
    }
}

/// Shared values for the publishing tests.
enum PublishingFixtures {
    /// 64 hex digits, all "ab".
    static let publicKeyA = String(repeating: "ab", count: 32)
    /// 64 hex digits, all "cd".
    static let publicKeyB = String(repeating: "cd", count: 32)

    /// Throwaway defaults domain so tests never touch the app domain.
    static func defaults() -> UserDefaults {
        guard let defaults = UserDefaults(suiteName: "odc.publishing.tests.\(UUID().uuidString)") else {
            fatalError("cannot create a scratch UserDefaults suite")
        }
        return defaults
    }
}
