import Foundation

/// Builds the document the entry reader loads, and strips the small amount of
/// markup an entry must not be allowed to run.
///
/// Everything here is pure string work, so it can be unit-tested without WebKit.
public enum EntryHTML {
    /// Wraps an entry's `raw.html` in a full document carrying the reader's
    /// default styles. A `<style>` block inside the entry is kept as-is: that is
    /// the dictionary's own typography.
    public static func document(html: String) -> String {
        let body = sanitized(html)
        return """
        <!DOCTYPE html>
        <html>
        <head>
        <meta charset="utf-8">
        <meta name="viewport" content="width=device-width, initial-scale=1">
        <style>
        \(defaultStylesheet)
        </style>
        </head>
        <body>
        \(body)
        </body>
        </html>
        """
    }

    /// Reader defaults: light/dark colour schemes, the system font stack, and
    /// containers (images, tables) that cannot overflow the window.
    static let defaultStylesheet = """
    :root {
      color-scheme: light dark;
      -webkit-text-size-adjust: 100%;
    }
    body {
      margin: 16px;
      font-family: -apple-system, BlinkMacSystemFont, "Helvetica Neue", "Hiragino Sans", sans-serif;
      font-size: 16px;
      line-height: 1.5;
      color: #1c1c1e;
      background: #ffffff;
      overflow-wrap: break-word;
    }
    a { color: #0a66c2; }
    img { max-width: 100%; height: auto; }
    table { max-width: 100%; border-collapse: collapse; word-break: break-word; }
    td, th { word-break: break-word; vertical-align: top; }
    pre, code { white-space: pre-wrap; word-break: break-word; }
    @media (prefers-color-scheme: dark) {
      body { color: #f2f2f7; background: #1c1c1e; }
      a { color: #6cb4ff; }
    }
    """

    /// Removes `<script>` elements and inline `on*` handlers. Comments are set
    /// aside first so a stray tag inside one is never treated as markup, and
    /// `<style>` blocks are left untouched.
    public static func sanitized(_ html: String) -> String {
        var comments: [String] = []
        var text = replacingMatches(in: html, pattern: "<!--[\\s\\S]*?-->") { match in
            comments.append(match)
            return placeholder(forComment: comments.count - 1)
        }
        // The self-closing form has to go first: the paired pattern below would
        // otherwise swallow everything up to the next `</script>`.
        text = replacingMatches(
            in: text,
            pattern: "<script\\b[^>]*/>",
            options: [.caseInsensitive]
        ) { _ in "" }
        text = replacingMatches(
            in: text,
            pattern: "<script\\b[^>]*>[\\s\\S]*?</script\\s*>",
            options: [.caseInsensitive]
        ) { _ in "" }
        text = replacingMatches(
            in: text,
            pattern: eventAttributePattern,
            options: [.caseInsensitive]
        ) { _ in "" }
        for (index, comment) in comments.enumerated() {
            text = text.replacingOccurrences(of: placeholder(forComment: index), with: comment)
        }
        return text
    }

    /// Target key of an `entry://` link: scheme dropped, percent-decoded,
    /// fragment and query removed. Case is preserved. Returns `nil` when the
    /// link carries no key.
    public static func entryTarget(from url: URL) -> String? {
        let prefix = "entry://"
        let absolute = url.absoluteString
        guard absolute.count > prefix.count,
              absolute.prefix(prefix.count).lowercased() == prefix
        else { return nil }
        var target = String(absolute.dropFirst(prefix.count))
        if let fragment = target.firstIndex(of: "#") {
            target = String(target[..<fragment])
        }
        if let query = target.firstIndex(of: "?") {
            target = String(target[..<query])
        }
        if let decoded = target.removingPercentEncoding {
            target = decoded
        }
        return target.isEmpty ? nil : target
    }

    /// Event handler attributes, matched by name: a looser `on[a-z]+` pattern
    /// would also eat ordinary attributes such as `only="…"`.
    private static let eventNames = [
        "abort", "afterprint", "animationend", "animationiteration", "animationstart",
        "beforeprint", "beforeunload", "blur", "canplay", "canplaythrough", "change",
        "click", "contextmenu", "copy", "cut", "dblclick", "drag", "dragend",
        "dragenter", "dragleave", "dragover", "dragstart", "drop", "durationchange",
        "ended", "error", "focus", "focusin", "focusout", "hashchange", "input",
        "invalid", "keydown", "keypress", "keyup", "load", "loadeddata",
        "loadedmetadata", "loadstart", "message", "mousedown", "mouseenter",
        "mouseleave", "mousemove", "mouseout", "mouseover", "mouseup", "mousewheel",
        "offline", "online", "pagehide", "pageshow", "paste", "pause", "play",
        "pointercancel", "pointerdown", "pointerenter", "pointerleave", "pointermove",
        "pointerout", "pointerover", "pointerup", "popstate", "progress",
        "ratechange", "reset", "resize", "scroll", "seeked", "seeking", "select",
        "stalled", "storage", "submit", "suspend", "timeupdate", "toggle",
        "touchcancel", "touchend", "touchmove", "touchstart", "transitionend",
        "transitionstart", "unload", "volumechange", "waiting", "wheel",
    ]

    private static let eventAttributePattern: String = {
        let alternatives = eventNames.sorted { $0.count > $1.count }.joined(separator: "|")
        let value = "(?:\"[^\"]*\"|'[^']*'|[^\\s>]+)"
        return "\\s+on(?:\(alternatives))\\s*=\\s*\(value)"
    }()

    private static func placeholder(forComment index: Int) -> String {
        "\u{0}\(index)\u{0}"
    }

    /// Applies `transform` to every match, keeping everything around it intact.
    private static func replacingMatches(
        in text: String,
        pattern: String,
        options: NSRegularExpression.Options = [],
        transform: (String) -> String
    ) -> String {
        guard let regex = try? NSRegularExpression(pattern: pattern, options: options) else {
            return text
        }
        let searchRange = NSRange(text.startIndex..<text.endIndex, in: text)
        var result = ""
        var cursor = text.startIndex
        regex.enumerateMatches(in: text, options: [], range: searchRange) { match, _, _ in
            guard let match, let matchRange = Range(match.range, in: text) else { return }
            result += text[cursor..<matchRange.lowerBound]
            result += transform(String(text[matchRange]))
            cursor = matchRange.upperBound
        }
        result += text[cursor...]
        return result
    }
}
