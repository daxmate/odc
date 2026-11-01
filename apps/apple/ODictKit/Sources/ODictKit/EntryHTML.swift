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
        document(html: html, stylesheets: [])
    }

    /// Wraps an entry in a document carrying the reader defaults followed by
    /// the dictionary's own sheets.
    ///
    /// The defaults come first and the dictionary's sheets after them, so a
    /// dictionary rule wins at equal specificity without having to out-specify
    /// the reader.
    public static func document(html: String, stylesheets: [LinkedStylesheet]) -> String {
        let body = sanitized(html)
        let entryStyles = stylesheets.map(styleElement(for:)).joined(separator: "\n")
        return """
        <!DOCTYPE html>
        <html>
        <head>
        <meta charset="utf-8">
        <meta name="viewport" content="width=device-width, initial-scale=1">
        <style>
        \(defaultStylesheet)
        </style>
        \(entryStyles)
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


    /// Routes every relative `src`/`href` and CSS `url(...)` reference to
    /// the `odc-resource` scheme.
    ///
    /// Absolute URLs, fragments and schemes the entry may legitimately carry
    /// are left untouched (see `ODCResource.url(for:)`); MDD keys written with
    /// backslashes or a leading slash are normalised on the way through.
    public static func rewriteResources(_ html: String) -> String {
        let attributePattern = "(?i)\\b(?:src|href)\\s*=\\s*(?:\"[^\"]*\"|'[^']*')"
        let withAttributes = replacingMatches(in: html, pattern: attributePattern) { match in
            rewriteAttribute(match)
        }
        let cssPattern = "(?i)url\\(\\s*(?:\"[^\"]*\"|'[^']*'|[^)]*?)\\s*\\)"
        return replacingMatches(in: withAttributes, pattern: cssPattern) { match in
            rewriteCSSURL(match)
        }
    }

    /// Rewrites one `src="..."` / `href='...'` match, preserving quotes.
    private static func rewriteAttribute(_ match: String) -> String {
        guard let equals = match.firstIndex(of: "=") else { return match }
        let prefix = String(match[match.startIndex...equals])
        let value = unquoted(String(match[match.index(after: equals)...]))
        guard let url = ODCResource.url(for: value) else { return match }
        return prefix + "\"" + url.absoluteString + "\""
    }

    /// Rewrites one `url(...)` token, keeping the author's quoting.
    private static func rewriteCSSURL(_ match: String) -> String {
        guard let open = match.firstIndex(of: "("),
              let close = match.lastIndex(of: ")"), open < close
        else { return match }
        let raw = String(match[match.index(after: open)..<close])
        let trimmed = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let url = ODCResource.url(for: unquoted(trimmed)) else { return match }
        let quote = trimmed.hasPrefix("'") ? "'" : (trimmed.hasPrefix("\"") ? "\"" : "")
        return "url(\(quote)\(url.absoluteString)\(quote))"
    }

    /// The value inside `"..."` or `'...'`, or the value itself.
    private static func unquoted(_ raw: String) -> String {
        let text = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        guard text.count >= 2 else { return text }
        if (text.hasPrefix("\"") && text.hasSuffix("\"")) ||
            (text.hasPrefix("'") && text.hasSuffix("'")) {
            return String(text.dropFirst().dropLast())
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

    // MARK: - External stylesheets

    /// Most CSS the reader will inline for one entry, so a pathological
    /// dictionary cannot stall the web view on a multi-megabyte sheet.
    public static let stylesheetByteLimit = 2 * 1024 * 1024

    /// One stylesheet the entry links to, already read out of the repository.
    public struct LinkedStylesheet: Equatable, Sendable {
        /// Resource key the sheet was stored under.
        public let key: String
        /// CSS text, with its relative references already rewritten.
        public let css: String

        public init(key: String, css: String) {
            self.key = key
            self.css = css
        }
    }

    /// Result of inlining an entry's external stylesheets.
    public struct InlinedStylesheets: Equatable, Sendable {
        /// Entry markup with the `<link>` elements it consumed removed.
        public let html: String
        /// Sheets to place after the reader defaults, in document order.
        public let stylesheets: [LinkedStylesheet]
        /// Keys that were missing, unreadable or over the size budget.
        public let skipped: [String]
        /// Bytes of CSS actually inlined.
        public let inlinedBytes: Int
    }

    /// Keys of the external stylesheets `html` links to, in document order.
    public static func stylesheetKeys(in html: String) -> [String] {
        var keys: [String] = []
        forEachMatch(in: html, pattern: "<link\\b[^>]*>") { tag in
            guard isStylesheetLink(tag), let href = attribute("href", in: tag) else { return }
            let key = ODCResource.normalizedKey(href)
            if !key.isEmpty, !keys.contains(key) {
                keys.append(key)
            }
        }
        return keys
    }

    /// Replaces every external stylesheet with the CSS stored for it.
    ///
    /// A sheet that is absent, unreadable or over `limit` is dropped and
    /// recorded in `skipped`; the entry still renders with the reader's
    /// defaults, so a broken dictionary degrades instead of failing.
    public static func inlineStylesheets(
        in html: String,
        limit: Int = EntryHTML.stylesheetByteLimit,
        resource: (String) -> Data?
    ) -> InlinedStylesheets {
        var sheets: [LinkedStylesheet] = []
        var skipped: [String] = []
        var inlinedBytes = 0
        var remaining = limit

        let body = replacingMatches(
            in: html,
            pattern: "<link\\b[^>]*>",
            options: [.caseInsensitive]
        ) { tag in
            guard isStylesheetLink(tag), let href = attribute("href", in: tag) else {
                return tag
            }
            let key = ODCResource.normalizedKey(href)
            guard !key.isEmpty else { return "" }
            guard let data = resource(key), let css = String(data: data, encoding: .utf8) else {
                skipped.append(key)
                return ""
            }
            let rewritten = rewriteStyleSheet(css)
            let cost = rewritten.utf8.count
            guard cost <= remaining else {
                skipped.append(key)
                return ""
            }
            remaining -= cost
            inlinedBytes += cost
            sheets.append(LinkedStylesheet(key: key, css: rewritten))
            return ""
        }

        return InlinedStylesheets(
            html: body,
            stylesheets: sheets,
            skipped: skipped,
            inlinedBytes: inlinedBytes
        )
    }

    /// Rewrites `url(...)` and quoted `@import` references in one sheet.
    public static func rewriteStyleSheet(_ css: String) -> String {
        let withImports = replacingMatches(
            in: css,
            pattern: "(?i)@import\\s+(\"[^\"]*\"|'[^']*')"
        ) { match in
            let raw = unquoted(String(match.dropFirst(7)))
            guard let url = ODCResource.url(for: raw) else { return match }
            return "@import url(\(url.absoluteString))"
        }
        return replacingMatches(in: withImports, pattern: "(?i)url\\(\\s*(?:\"[^\"]*\"|'[^']*'|[^)]*?)\\s*\\)") { match in
            rewriteCSSURL(match)
        }
    }

    /// True when a `<link>` tag carries the reader a stylesheet.
    private static func isStylesheetLink(_ tag: String) -> Bool {
        guard let rel = attribute("rel", in: tag) else { return false }
        return rel.lowercased().split(whereSeparator: { $0 == " " || $0 == "\t" }).contains("stylesheet")
    }

    /// Value of one attribute in `tag`, without its quotes.
    private static func attribute(_ name: String, in tag: String) -> String? {
        let pattern = "(?i)\\b" + name + "\\s*=\\s*(\"[^\"]*\"|'[^']*'|[^\\s>]+)"
        guard let match = firstMatch(in: tag, pattern: pattern),
              let separator = match.firstIndex(of: "=")
        else { return nil }
        let value = match[match.index(after: separator)...]
            .trimmingCharacters(in: .whitespacesAndNewlines)
        return unquoted(String(value))
    }

    /// Calls `body` for every match of `pattern` in `text`.
    private static func forEachMatch(
        in text: String,
        pattern: String,
        options: NSRegularExpression.Options = [.caseInsensitive],
        _ body: (String) -> Void
    ) {
        guard let regex = try? NSRegularExpression(pattern: pattern, options: options) else { return }
        let range = NSRange(text.startIndex..., in: text)
        for match in regex.matches(in: text, range: range) {
            guard let span = Range(match.range, in: text) else { continue }
            body(String(text[span]))
        }
    }

    /// First match of `pattern` in `text`, as a `String`.
    private static func firstMatch(in text: String, pattern: String, options: NSRegularExpression.Options = [.caseInsensitive]) -> Substring? {
        guard let regex = try? NSRegularExpression(pattern: pattern, options: options) else { return nil }
        let range = NSRange(text.startIndex..., in: text)
        guard let match = regex.firstMatch(in: text, range: range),
              let span = Range(match.range, in: text)
        else { return nil }
        return text[span]
    }

    /// Escapes text placed inside a `<style>` element.
    private static func embeddableStyle(_ css: String) -> String {
        css.replacingOccurrences(
            of: "</style",
            with: "<\\/style",
            options: [.caseInsensitive]
        )
    }

    /// Wraps one sheet in its own `<style>` element.
    private static func styleElement(for sheet: LinkedStylesheet) -> String {
        let key = sheet.key
            .replacingOccurrences(of: "&", with: "&amp;")
            .replacingOccurrences(of: "\"", with: "&quot;")
        return "<style data-odc-sheet=\"\(key)\">\n\(embeddableStyle(sheet.css))\n</style>"
    }

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
