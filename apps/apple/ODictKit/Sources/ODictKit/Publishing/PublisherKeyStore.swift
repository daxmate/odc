import Foundation
import Security

/// Problems raised while storing or reading the publisher keypair.
public enum PublisherKeyStoreError: LocalizedError, Equatable {
    /// The PEM the caller handed over was blank.
    case emptyPEM
    /// The stored metadata could not be read back as an identity.
    case malformedIdentity(String)
    /// A Security framework call returned an error status.
    case keychain(OSStatus)

    public var errorDescription: String? {
        switch self {
        case .emptyPEM:
            return "私钥内容为空。"
        case .malformedIdentity(let detail):
            return "钥匙串中的发布者信息无法解析：\(detail)"
        case .keychain(let status):
            let message = SecCopyErrorMessageString(status, nil) as String?
            return "钥匙串操作失败（\(status)）：\(message ?? "未知错误")"
        }
    }
}

/// Where the app keeps the publisher private key.
///
/// The private PEM only ever lives in the keychain; implementations must not
/// write it to UserDefaults, files or logs.
@MainActor
public protocol PublisherKeyStore: AnyObject {
    /// Public metadata for the stored keypair, or nil when none is stored.
    func load() throws -> PublisherIdentity?
    /// Store (or replace) the keypair.
    func save(pem: String, identity: PublisherIdentity) throws
    /// The PKCS#8 PEM private key, or nil when none is stored.
    func privateKeyPEM() throws -> String?
    /// Remove both halves of the stored keypair.
    func delete() throws
}

/// Keychain-backed store: one generic-password item for the PEM and one for
/// the public metadata JSON.
///
/// macOS uses the login keychain, iOS the data-protection keychain; both need
/// no extra entitlement for items owned by the app itself. The accessibility
/// classes follow the platform conventions: readable while unlocked on macOS,
/// readable after first unlock on iOS so background work can still sign.
@MainActor
public final class KeychainPublisherKeyStore: PublisherKeyStore {
    /// Keychain service name shared by both items.
    public static let service = "com.daxmate.odict.publisher"
    /// Account holding the PKCS#8 PEM private key.
    public static let privateKeyAccount = "default"
    /// Account holding the identity JSON.
    public static let metadataAccount = "metadata"

    private let service: String

    public init(service: String = KeychainPublisherKeyStore.service) {
        self.service = service
    }

    public func load() throws -> PublisherIdentity? {
        guard let data = try copyData(account: Self.metadataAccount) else { return nil }
        do {
            return try JSONDecoder().decode(PublisherIdentity.self, from: data)
        } catch {
            throw PublisherKeyStoreError.malformedIdentity(String(describing: error))
        }
    }

    public func save(pem: String, identity: PublisherIdentity) throws {
        guard !pem.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            throw PublisherKeyStoreError.emptyPEM
        }
        let metadata = try JSONEncoder().encode(identity)
        try write(account: Self.privateKeyAccount, data: Data(pem.utf8))
        do {
            try write(account: Self.metadataAccount, data: metadata)
        } catch {
            // Do not leave a key without the metadata that names it.
            try? deleteItem(account: Self.privateKeyAccount)
            throw error
        }
    }

    public func privateKeyPEM() throws -> String? {
        guard let data = try copyData(account: Self.privateKeyAccount) else { return nil }
        guard let pem = String(data: data, encoding: .utf8) else {
            throw PublisherKeyStoreError.malformedIdentity("私钥不是 UTF-8 文本")
        }
        return pem
    }

    public func delete() throws {
        try deleteItem(account: Self.metadataAccount)
        try deleteItem(account: Self.privateKeyAccount)
    }

    // MARK: - Security plumbing

    private static var accessibility: CFString {
        #if os(macOS)
        kSecAttrAccessibleWhenUnlocked
        #else
        kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        #endif
    }

    private func baseQuery(account: String) -> [String: Any] {
        [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
        ]
    }

    private func copyData(account: String) throws -> Data? {
        var query = baseQuery(account: account)
        query[kSecReturnData as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitOne
        var item: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &item)
        switch status {
        case errSecSuccess:
            return item as? Data
        case errSecItemNotFound:
            return nil
        default:
            throw PublisherKeyStoreError.keychain(status)
        }
    }

    private func write(account: String, data: Data) throws {
        let query = baseQuery(account: account)
        let update: [String: Any] = [kSecValueData as String: data]
        let updateStatus = SecItemUpdate(query as CFDictionary, update as CFDictionary)
        switch updateStatus {
        case errSecSuccess:
            return
        case errSecItemNotFound:
            var insert = query
            insert[kSecValueData as String] = data
            insert[kSecAttrAccessible as String] = Self.accessibility
            let addStatus = SecItemAdd(insert as CFDictionary, nil)
            guard addStatus == errSecSuccess else {
                throw PublisherKeyStoreError.keychain(addStatus)
            }
        default:
            throw PublisherKeyStoreError.keychain(updateStatus)
        }
    }

    private func deleteItem(account: String) throws {
        let status = SecItemDelete(baseQuery(account: account) as CFDictionary)
        guard status == errSecSuccess || status == errSecItemNotFound else {
            throw PublisherKeyStoreError.keychain(status)
        }
    }
}
