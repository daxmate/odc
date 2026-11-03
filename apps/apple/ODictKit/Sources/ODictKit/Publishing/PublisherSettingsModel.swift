import Foundation
import Observation

/// State behind the publisher settings screen.
///
/// Keeps the keychain calls (and their errors) out of the view so the same
/// logic is testable with an in-memory store.
@MainActor
@Observable
public final class PublisherSettingsModel {
    /// Stored keypair metadata, or nil when nothing is configured.
    public private(set) var identity: PublisherIdentity?
    /// Last failure, shown inline; cleared on the next attempt.
    public private(set) var errorMessage: String?
    /// True while a keychain or crypto call runs.
    public private(set) var isBusy = false
    /// Name the user wants on the next generate or import.
    public var name: String = ""

    @ObservationIgnored private let keyStore: PublisherKeyStore
    @ObservationIgnored private let fallbackName: String

    /// - Parameters:
    ///   - keyStore: storage for the PEM and metadata; injectable for tests.
    ///   - fallbackName: name used when the field is blank.
    public init(keyStore: PublisherKeyStore = KeychainPublisherKeyStore(), fallbackName: String = NSUserName()) {
        self.keyStore = keyStore
        self.fallbackName = fallbackName
        reload()
    }

    /// True when a keypair is stored.
    public var hasIdentity: Bool { identity != nil }

    /// Fingerprint of the stored key, or an empty string.
    public var fingerprint: String { identity?.fingerprint ?? "" }

    /// Public key in the form users copy out, or an empty string.
    public var publicKeyHex: String { identity?.publicKeyHex ?? "" }

    /// Re-read the stored keypair.
    public func reload() {
        do {
            identity = try keyStore.load()
            if name.isEmpty, let identity {
                name = identity.name
            }
        } catch {
            identity = nil
            errorMessage = error.localizedDescription
        }
    }

    /// Generate a fresh Ed25519 keypair and store it, replacing any existing
    /// one. The caller is expected to confirm before replacing.
    public func generate() {
        guard !isBusy else { return }
        isBusy = true
        defer { isBusy = false }
        errorMessage = nil
        do {
            let keypair = try ODCPublishing.generateKeypair()
            let resolvedName = resolvedIdentityName()
            let newIdentity = PublisherIdentity(name: resolvedName, publicKeyHex: keypair.publicHex)
            try keyStore.save(pem: keypair.pem, identity: newIdentity)
            identity = newIdentity
            name = resolvedName
        } catch {
            errorMessage = error.localizedDescription
        }
    }

    /// Validate and store a PEM the user picked from disk.
    public func importKey(pem: String) {
        guard !isBusy else { return }
        isBusy = true
        defer { isBusy = false }
        errorMessage = nil
        do {
            let publicHex = try ODCPublishing.publicKeyHex(forPEM: pem)
            let resolvedName = resolvedIdentityName()
            let newIdentity = PublisherIdentity(
                name: resolvedName,
                publicKeyHex: publicHex,
                createdAt: identity?.createdAt ?? Date()
            )
            try keyStore.save(pem: pem, identity: newIdentity)
            identity = newIdentity
            name = resolvedName
        } catch {
            errorMessage = error.localizedDescription
        }
    }

    /// Remove the stored keypair. Published artifacts are left alone.
    public func delete() {
        guard !isBusy else { return }
        isBusy = true
        defer { isBusy = false }
        errorMessage = nil
        do {
            try keyStore.delete()
            identity = nil
        } catch {
            errorMessage = error.localizedDescription
        }
    }

    /// Drop the inline error, for example when the sheet is dismissed.
    public func clearError() {
        errorMessage = nil
    }

    private func resolvedIdentityName() -> String {
        let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        if !trimmed.isEmpty { return trimmed }
        if let existing = identity?.name, !existing.isEmpty { return existing }
        let fallback = fallbackName.trimmingCharacters(in: .whitespacesAndNewlines)
        return fallback.isEmpty ? "发布者" : fallback
    }
}
