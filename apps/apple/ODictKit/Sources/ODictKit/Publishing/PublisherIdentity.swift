import CryptoKit
import Foundation

/// A publisher keypair as the app remembers it.
///
/// The private half never appears here: it stays in the keychain behind
/// `PublisherKeyStore`, and only the public metadata travels through
/// `Codable`. That keeps logged or archived payloads free of PEM blocks.
public struct PublisherIdentity: Codable, Equatable, Sendable {
    /// Display name the user chose when the key was created or imported.
    public let name: String
    /// Lower-case hex public key, 64 characters.
    public let publicKeyHex: String
    /// When the app first stored the key.
    public let createdAt: Date

    public init(name: String, publicKeyHex: String, createdAt: Date = Date()) {
        self.name = name
        self.publicKeyHex = publicKeyHex
        self.createdAt = createdAt
    }

    /// Stable display fingerprint: SHA-256 over the raw 32 key bytes.
    ///
    /// The first 16 digest bytes are shown as eight groups of four upper-case
    /// hex digits ("AB12 CD34 EF56 7890 ..."). The fingerprint only identifies
    /// a key on screen; it is not a security boundary. Keys that are not valid
    /// hex fall back to hashing the text so the UI still has something stable
    /// to show.
    public var fingerprint: String {
        let digest = SHA256.hash(data: Self.fingerprintInput(publicKeyHex))
        let hex = digest.prefix(16).map { String(format: "%02X", $0) }.joined()
        let characters = Array(hex)
        var groups: [String] = []
        var index = 0
        while index < characters.count {
            let end = min(index + 4, characters.count)
            groups.append(String(characters[index..<end]))
            index = end
        }
        return groups.joined(separator: " ")
    }

    /// First two fingerprint groups, for tight rows and menu titles.
    public var shortFingerprint: String {
        fingerprint.split(separator: " ").prefix(2).joined(separator: " ")
    }

    /// True when `hex` is a 64-character hexadecimal Ed25519 public key.
    public static func isValidPublicKeyHex(_ hex: String) -> Bool {
        hex.count == 64 && hex.allSatisfy(\.isHexDigit)
    }

    /// Normalises a pasted key: trims whitespace and lower-cases the hex.
    public static func normalizedPublicKeyHex(_ hex: String) -> String {
        hex.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
    }

    private static func fingerprintInput(_ hex: String) -> Data {
        let normalised = normalizedPublicKeyHex(hex)
        var bytes: [UInt8] = []
        bytes.reserveCapacity(normalised.count / 2)
        var index = normalised.startIndex
        while index < normalised.endIndex {
            let next = normalised.index(index, offsetBy: 2, limitedBy: normalised.endIndex) ?? normalised.endIndex
            guard let byte = UInt8(normalised[index..<next], radix: 16) else {
                return Data(normalised.utf8)
            }
            bytes.append(byte)
            index = next
        }
        return Data(bytes)
    }
}
