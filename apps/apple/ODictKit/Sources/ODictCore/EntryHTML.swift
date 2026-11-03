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

    // MARK: - Single-page composition

    /// One dictionary's slice of the entry page.
    public struct PageSection: Equatable, Sendable {
        public let dictionaryID: String
        public let name: String
        public let html: String
        public let stylesheets: [LinkedStylesheet]

        public init(
            dictionaryID: String,
            name: String,
            html: String,
            stylesheets: [LinkedStylesheet] = []
        ) {
            self.dictionaryID = dictionaryID
            self.name = name
            self.html = html
            self.stylesheets = stylesheets
        }
    }

    /// Page chrome: the margins, the rule between dictionaries and the
    /// typography the shadow roots inherit from the page.
    static let pageStylesheet = """
    :root {
      color-scheme: light dark;
      -webkit-text-size-adjust: 100%;
    }
    html, body {
      margin: 0;
      padding: 0;
    }
    body {
      font-family: -apple-system, BlinkMacSystemFont, "Helvetica Neue", "Hiragino Sans", sans-serif;
      font-size: 16px;
      line-height: 1.5;
      color: #1c1c1e;
      background: #ffffff;
      overflow-wrap: break-word;
    }
    .odc-section {
      padding: 16px;
    }
    .odc-section + .odc-section {
      border-top: 1px solid rgba(60, 60, 67, 0.29);
    }
    .odc-dictionary {
      margin: 0 0 12px;
      font-size: 13px;
      font-weight: 600;
      line-height: 1.2;
      letter-spacing: 0.02em;
      color: rgba(60, 60, 67, 0.6);
    }
    @media (prefers-color-scheme: dark) {
      body { color: #f2f2f7; background: #000000; }
      .odc-section + .odc-section { border-top-color: rgba(84, 84, 88, 0.65); }
      .odc-dictionary { color: rgba(235, 235, 245, 0.6); }
    }
    """

    /// Reader defaults written for a shadow root: the host stands in for
    /// the page's html element, and .odc-body for its body.
    static let shadowDefaultStylesheet = """
    :host {
      display: block;
      color-scheme: light dark;
      -webkit-text-size-adjust: 100%;
      font-family: -apple-system, BlinkMacSystemFont, "Helvetica Neue", "Hiragino Sans", sans-serif;
      font-size: 16px;
      line-height: 1.5;
      color: #1c1c1e;
      overflow-wrap: break-word;
    }
    .odc-body {
      margin: 0;
    }
    a { color: #0a66c2; }
    img { max-width: 100%; height: auto; }
    table { max-width: 100%; border-collapse: collapse; word-break: break-word; }
    td, th { word-break: break-word; vertical-align: top; }
    pre, code { white-space: pre-wrap; word-break: break-word; }
    @media (prefers-color-scheme: dark) {
      :host { color: #f2f2f7; }
      a { color: #6cb4ff; }
    }
    """

    /// Builds one document for every dictionary that has an entry, so the web
    /// view scrolls the whole page itself.
    ///
    /// Each section carries its own declarative shadow root: a dictionary's
    /// CSS cannot reach its neighbours, and the page only holds the reader's
    /// chrome. The document holds no script.
    public static func page(_ sections: [PageSection]) -> String {
        let showsTitles = sections.count > 1
        let body = sections.enumerated()
            .map { sectionMarkup($1, index: $0, showsTitle: showsTitles) }
            .joined(separator: "\n")
        return """
        <!DOCTYPE html>
        <html>
        <head>
        <meta charset="utf-8">
        <meta name="viewport" content="width=device-width, initial-scale=1">
        <style>
        \(pageStylesheet)
        </style>
        </head>
        <body>
        \(body)
        </body>
        </html>
        """
    }


    /// Whether a section carries markup worth drawing. An entry with an empty
    /// or whitespace-only body falls back to its parsed fields instead.
    public static func hasMarkup(_ html: String?) -> Bool {
        guard let html else { return false }
        return !sanitized(html).trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    /// Rewrites one section's style blocks for its shadow root.
    private static func scopedStyleBlocks(in html: String) -> String {
        let pattern = "(?is)(<style\\b[^>]*>)([\\s\\S]*?)(</style\\s*>)"
        guard let regex = try? NSRegularExpression(pattern: pattern) else { return html }
        let range = NSRange(html.startIndex..., in: html)
        var result = ""
        var cursor = html.startIndex
        for match in regex.matches(in: html, range: range) {
            guard let whole = Range(match.range, in: html),
                  let open = Range(match.range(at: 1), in: html),
                  let css = Range(match.range(at: 2), in: html),
                  let close = Range(match.range(at: 3), in: html)
            else { continue }
            result += html[cursor..<whole.lowerBound]
            result += html[open]
            result += shadowCSS(String(html[css]))
            result += html[close]
            cursor = whole.upperBound
        }
        result += html[cursor...]
        return result
    }

    /// Rewrites a dictionary stylesheet for a shadow root: html and :root
    /// become :host, body becomes .odc-body.
    ///
    /// Rules that start with an at-rule (@media, @font-face, ...) keep their
    /// prelude; their contents are selectors again and are rewritten as usual.
    public static func shadowCSS(_ css: String) -> String {
        var result = ""
        var prelude = ""
        var index = css.startIndex
        while index < css.endIndex {
            let character = css[index]
            switch character {
            case "{":
                result += transformedPrelude(prelude) + "{"
                prelude = ""
            case "}":
                result += prelude + "}"
                prelude = ""
            default:
                prelude.append(character)
            }
            index = css.index(after: index)
        }
        result += prelude
        return result
    }

    private static func transformedPrelude(_ prelude: String) -> String {
        let trimmed = prelude.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty, !trimmed.hasPrefix("@") else { return prelude }
        return scopedSelector(prelude)
    }

    /// Maps the element names of one selector list onto the shadow root.
    private static func scopedSelector(_ selector: String) -> String {
        var result = ""
        var index = selector.startIndex
        var previous: Character = " "
        while index < selector.endIndex {
            let character = selector[index]
            guard character.isLetter || character == "-" || character == "_" else {
                result.append(character)
                if !character.isWhitespace { previous = character }
                index = selector.index(after: index)
                continue
            }
            var token = ""
            var cursor = index
            while cursor < selector.endIndex {
                let next = selector[cursor]
                guard next.isLetter || next.isNumber || next == "-" || next == "_" else { break }
                token.append(next)
                cursor = selector.index(after: cursor)
            }
            result += scopedToken(token, after: previous)
            previous = token.last ?? previous
            index = cursor
        }
        return result
    }

    /// .html, #body and [data-body] are names, not elements.
    private static func scopedToken(_ token: String, after previous: Character) -> String {
        if previous == ":", token.lowercased() == "root" { return "host" }
        guard !".[#-".contains(previous) else { return token }
        switch token.lowercased() {
        case "html": return ":host"
        case "body": return ".odc-body"
        default: return token
        }
    }

    /// Routes one section's resource references through that section, so the
    /// scheme handler can name the dictionary that asked for the payload.
    public static func rewriteResources(_ html: String, section: Int) -> String {
        let scheme = ODCResource.scheme + ":///"
        return rewriteResources(html)
            .replacingOccurrences(of: scheme, with: scheme + "s\(section)/")
    }

    /// Points entry:// links at the section they came from.
    public static func rewriteEntryLinks(in html: String, section: Int) -> String {
        let pattern = "(?i)href\\s*=\\s*(['\"])entry://([^'\"]*)\\1"
        guard let regex = try? NSRegularExpression(pattern: pattern) else { return html }
        let range = NSRange(html.startIndex..., in: html)
        var result = ""
        var cursor = html.startIndex
        for match in regex.matches(in: html, range: range) {
            guard let whole = Range(match.range, in: html),
                  let target = Range(match.range(at: 2), in: html)
            else { continue }
            result += html[cursor..<whole.lowerBound]
            result += "href=\"entry://s\(section)/\(html[target])\""
            cursor = whole.upperBound
        }
        result += html[cursor...]
        return result
    }

    /// Splits entry://s2/target into the section and the target.
    ///
    /// A link without a section (entry://target) keeps the older
    /// single-dictionary reading: (nil, "target").
    public static func addressedEntryTarget(from url: URL) -> (section: Int?, target: String)? {
        guard let raw = entryTarget(from: url) else { return nil }
        let parts = raw.split(separator: "/", maxSplits: 1, omittingEmptySubsequences: false)
        guard parts.count == 2, let section = sectionIndex(from: String(parts[0])) else {
            return (nil, raw)
        }
        let target = String(parts[1])
        return target.isEmpty ? nil : (section, target)
    }

    /// Parses the s<digits> prefix a section URL carries.
    static func sectionIndex(from component: String) -> Int? {
        guard component.count > 1, component.first?.lowercased() == "s" else { return nil }
        let digits = component.dropFirst()
        guard digits.allSatisfy({ $0.isNumber }) else { return nil }
        return Int(digits)
    }

    private static func escapeText(_ text: String) -> String {
        text
            .replacingOccurrences(of: "&", with: "&amp;")
            .replacingOccurrences(of: "<", with: "&lt;")
            .replacingOccurrences(of: ">", with: "&gt;")
            .replacingOccurrences(of: "\"", with: "&quot;")
    }

    private static func sectionMarkup(
        _ section: PageSection,
        index: Int,
        showsTitle: Bool
    ) -> String {
        var body = sanitized(section.html)
        body = scopedStyleBlocks(in: body)
        body = rewriteResources(body, section: index)
        body = rewriteEntryLinks(in: body, section: index)
        let title = showsTitle
            ? "<h2 class=\"odc-dictionary\">\(escapeText(section.name))</h2>\n"
            : ""
        let dictionaryStyles = section.stylesheets
            .map { shadowCSS(embeddableStyle($0.css)) }
            .joined(separator: "\n")
        return """
        <section class="odc-section">
        \(title)<template shadowrootmode="open">
        <style>
        \(shadowDefaultStylesheet)
        \(dictionaryStyles)
        </style>
        <div class="odc-body">
        \(body)
        </div>
        </template>
        </section>
        """
    }
}
