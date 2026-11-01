import Foundation

/// Rules shared by the entry HTML rewriter and the `odc-resource` URL scheme
/// handler.
///
/// Everything here is pure string work, so it can be unit-tested without
/// WebKit.
public enum ODCResource {
    /// Scheme the reader serves MDD payloads under.
    public static let scheme = "odc-resource"

    /// Base URL handed to the web view: every relative URL an entry carries
    /// resolves against it.
    public static let baseURL = URL(string: "odc-resource:///")!

    /// Keys the reader serves from the repository, extension to MIME type.
    private static let mimeTypes: [String: String] = [
        "png": "image/png",
        "jpg": "image/jpeg",
        "jpeg": "image/jpeg",
        "gif": "image/gif",
        "webp": "image/webp",
        "svg": "image/svg+xml",
        "mp3": "audio/mpeg",
        "wav": "audio/wav",
        "ogg": "audio/ogg",
        "m4a": "audio/mp4",
        "aac": "audio/aac",
        "woff": "font/woff",
        "woff2": "font/woff2",
        "ttf": "font/ttf",
        "css": "text/css",
        "js": "text/javascript",
    ]

    /// Content type for `key`; unknown extensions fall back to binary.
    public static func mimeType(for key: String) -> String {
        let expanded = (key as NSString).pathExtension.lowercased()
        return mimeTypes[expanded] ?? "application/octet-stream"
    }

    /// Canonical resource key: backslashes became slashes, leading `./` and
    /// `/` are dropped. Case is preserved; the core compares it loosely.
    public static func normalizedKey(_ raw: String) -> String {
        var key = raw.replacingOccurrences(of: "\\", with: "/")
        while key.hasPrefix("./") {
            key.removeFirst(2)
        }
        while key.hasPrefix("/") {
            key.removeFirst()
        }
        return key
    }

    /// URL for `raw`, or `nil` when it is not a relative resource path.
    ///
    /// Absolute URLs, fragments and the schemes an entry may legally carry
    /// (`http`, `https`, `data`, `mailto`, `tel`, `javascript`) are left to
    /// WebKit; only relative references are routed to the repository.
    public static func url(for raw: String) -> URL? {
        let trimmed = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty, !trimmed.hasPrefix("#") else { return nil }
        let lowered = trimmed.lowercased()
        // Dictionaries spell audio links `sound://hwd/ame/8/memory.mp3`; the
        // reader plays those from the repository, so the scheme is dropped.
        if lowered.hasPrefix("sound://") {
            let key = normalizedKey(String(trimmed.dropFirst("sound://".count)))
            return key.isEmpty ? nil : resourceURL(for: key)
        }
        for prefix in ["http://", "https://", "data:", "mailto:", "tel:", "javascript:",
                       "entry://", "odc-resource://", "file://", "about:", "ftp://"] {
            if lowered.hasPrefix(prefix) { return nil }
        }
        var path = trimmed
        if let cut = path.firstIndex(where: { $0 == "#" || $0 == "?" }) {
            path = String(path[path.startIndex..<cut])
        }
        let key = normalizedKey(path)
        guard !key.isEmpty else { return nil }
        return resourceURL(for: key)
    }

    /// `odc-resource` URL for a normalised key.
    private static func resourceURL(for key: String) -> URL? {
        var allowed = CharacterSet.urlPathAllowed
        allowed.remove(charactersIn: "?#%")
        let encoded = key.addingPercentEncoding(withAllowedCharacters: allowed) ?? key
        return URL(string: "\(scheme):///\(encoded)")
    }

    /// Whether the reader plays `key` itself instead of navigating to it.
    public static func isAudio(_ key: String) -> Bool {
        let expanded = (key as NSString).pathExtension.lowercased()
        return ["mp3", "wav", "ogg", "m4a", "aac"].contains(expanded)
    }

    /// Key carried by a request for one of our URLs.
    ///
    /// Both `odc-resource:///pic/a.png` and `odc-resource://pic/a.png`
    /// resolve to `pic/a.png`.
    public static func key(from url: URL) -> String? {
        guard url.scheme?.lowercased() == scheme else { return nil }
        let host = url.host() ?? ""
        let path = url.path(percentEncoded: false)
        let joined = host.isEmpty ? path : host + path
        let key = normalizedKey(joined)
        return key.isEmpty ? nil : key
    }

    /// Splits one of our URLs into the section that asked for the payload and
    /// the repository key. odc-resource:///s2/img/a.png resolves to
    /// (2, "img/a.png"); a link without a section resolves with nil, so a page
    /// that carries a single dictionary keeps working.
    public static func address(from url: URL) -> (section: Int?, key: String)? {
        guard let key = key(from: url) else { return nil }
        let parts = key.split(separator: "/", maxSplits: 1, omittingEmptySubsequences: false)
        guard parts.count == 2, let section = EntryHTML.sectionIndex(from: String(parts[0])) else {
            return (nil, key)
        }
        let remainder = String(parts[1])
        return remainder.isEmpty ? nil : (section, remainder)
    }
}
