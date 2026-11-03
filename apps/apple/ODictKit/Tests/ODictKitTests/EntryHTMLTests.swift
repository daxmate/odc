import Foundation
import XCTest
@testable import ODictCore
@testable import ODictKit

final class EntryHTMLTests: XCTestCase {
    // MARK: - document(html:)

    func testDocumentHasDoctypeCharsetAndViewport() {
        let document = EntryHTML.document(html: "<p>hello</p>")
        XCTAssertTrue(document.hasPrefix("<!DOCTYPE html>"))
        XCTAssertTrue(document.contains("charset=\"utf-8\""))
        XCTAssertTrue(document.contains("name=\"viewport\""))
    }

    func testDocumentCarriesTheDefaultStylesheet() {
        let document = EntryHTML.document(html: "")
        XCTAssertTrue(document.contains("color-scheme: light dark"))
        XCTAssertTrue(document.contains("-apple-system"))
        XCTAssertTrue(document.contains("line-height: 1.5"))
        XCTAssertTrue(document.contains("margin: 16px"))
        XCTAssertTrue(document.contains("max-width: 100%"))
        XCTAssertTrue(document.contains("prefers-color-scheme: dark"))
        XCTAssertTrue(document.contains("word-break"))
    }

    func testDocumentKeepsTheEntryMarkup() {
        let entry = "<div class=\"entry\"><p>apple</p></div>"
        let document = EntryHTML.document(html: entry)
        XCTAssertTrue(document.contains(entry))
    }

    func testDocumentKeepsTheEntryStyleBlock() {
        let entry = "<style>.entry { color: red; }</style><p class=\"entry\">apple</p>"
        let document = EntryHTML.document(html: entry)
        XCTAssertTrue(document.contains("<style>.entry { color: red; }</style>"))
    }

    func testDocumentDropsScriptsOnTheWayIn() {
        let document = EntryHTML.document(html: "<p>apple</p><script>alert(1)</script>")
        XCTAssertFalse(document.contains("alert(1)"))
        XCTAssertTrue(document.contains("<p>apple</p>"))
    }

    // MARK: - External stylesheets

    func testStylesheetKeysReadsOnlyStylesheetLinks() {
        let html = "<link rel=\"icon\" href=\"favicon.ico\"><link rel=\"stylesheet\" href=\"./style.css\"><LINK REL=\"StyleSheet\" HREF=\"print.css\">"
        XCTAssertEqual(EntryHTML.stylesheetKeys(in: html), ["style.css", "print.css"])
    }

    func testInlineStylesheetsReadsTheLinkedSheet() {
        let css = "body { color: red }"
        let result = EntryHTML.inlineStylesheets(in: "<link rel=\"stylesheet\" href=\"style.css\"><p>a</p>") { key in
            key == "style.css" ? Data(css.utf8) : nil
        }

        XCTAssertEqual(result.stylesheets.map(\.key), ["style.css"])
        XCTAssertEqual(result.stylesheets.first?.css, css)
        XCTAssertEqual(result.inlinedBytes, css.utf8.count)
        XCTAssertTrue(result.skipped.isEmpty)
        XCTAssertFalse(result.html.contains("<link"))
        XCTAssertTrue(result.html.contains("<p>a</p>"))
    }

    func testInlineStylesheetsKeepsDocumentOrder() {
        let html = "<link rel=\"stylesheet\" href=\"one.css\"><link rel=\"stylesheet\" href=\"two.css\">"
        let result = EntryHTML.inlineStylesheets(in: html) { key in
            Data("/* \(key) */".utf8)
        }

        XCTAssertEqual(result.stylesheets.map(\.key), ["one.css", "two.css"])
    }

    func testInlineStylesheetsSkipsMissingSheets() {
        let result = EntryHTML.inlineStylesheets(in: "<link rel=\"stylesheet\" href=\"missing.css\"><p>a</p>") { _ in nil }

        XCTAssertTrue(result.stylesheets.isEmpty)
        XCTAssertEqual(result.skipped, ["missing.css"])
        XCTAssertEqual(result.inlinedBytes, 0)
        XCTAssertFalse(result.html.contains("<link"))
        XCTAssertTrue(result.html.contains("<p>a</p>"))
    }

    func testInlineStylesheetsRewritesURLsAndImports() {
        let css = "@import \"base.css\";\n"
            + ".a { background: url(img/bg.png) }\n"
            + ".b { background: url(\"img/bg.png\") }"
        let result = EntryHTML.inlineStylesheets(in: "<link rel=\"stylesheet\" href=\"style.css\">") { _ in
            Data(css.utf8)
        }

        let inlined = result.stylesheets.first?.css ?? ""
        XCTAssertTrue(inlined.contains("@import url(odc-resource:///base.css)"), inlined)
        XCTAssertTrue(inlined.contains("url(odc-resource:///img/bg.png)"), inlined)
        XCTAssertFalse(inlined.contains("url(\"img/bg.png\")"), inlined)
    }

    func testInlineStylesheetsHonoursTheBudget() {
        let html = "<link rel=\"stylesheet\" href=\"big.css\"><link rel=\"stylesheet\" href=\"small.css\">"
        let result = EntryHTML.inlineStylesheets(in: html, limit: 20) { key in
            key == "big.css" ? Data(repeating: 0x61, count: 64) : Data("p{a}".utf8)
        }

        XCTAssertEqual(result.stylesheets.map(\.key), ["small.css"])
        XCTAssertEqual(result.skipped, ["big.css"])
        XCTAssertEqual(result.inlinedBytes, 4)
    }

    func testInlineStylesheetsNeutralisesClosingStyleTags() {
        let css = "p { content: \"</style><script>alert(1)</script>\" }"
        let result = EntryHTML.inlineStylesheets(in: "<link rel=\"stylesheet\" href=\"style.css\">") { _ in
            Data(css.utf8)
        }
        let document = EntryHTML.document(html: result.html, stylesheets: result.stylesheets)

        XCTAssertFalse(document.contains("</style><script>"))
        XCTAssertTrue(document.contains("<\\/style>"))
    }

    func testDocumentPlacesDictionarySheetsAfterTheDefaults() {
        let document = EntryHTML.document(
            html: "<p>a</p>",
            stylesheets: [EntryHTML.LinkedStylesheet(key: "style.css", css: ".entry { color: red }")]
        )

        let defaults = try? XCTUnwrap(document.range(of: "font-family: -apple-system"))
        let dictionary = try? XCTUnwrap(document.range(of: ".entry { color: red }"))
        XCTAssertNotNil(defaults)
        XCTAssertNotNil(dictionary)
        if let defaults, let dictionary {
            XCTAssertLessThan(defaults.lowerBound, dictionary.lowerBound)
        }
        XCTAssertTrue(document.contains("data-odc-sheet=\"style.css\""))
    }

    // MARK: - sanitized(_:)

    func testSanitizedRemovesScriptElements() {
        let html = "<p>a</p><script>alert(1)</script><p>b</p>"
        XCTAssertEqual(EntryHTML.sanitized(html), "<p>a</p><p>b</p>")
    }

    func testSanitizedIgnoresScriptCase() {
        let html = "<p>a</p><SCRIPT>alert(1)</SCRIPT>"
        XCTAssertEqual(EntryHTML.sanitized(html), "<p>a</p>")
    }

    func testSanitizedRemovesScriptWithSource() {
        let html = "<p>a</p><script src=\"evil.js\"></script>"
        XCTAssertEqual(EntryHTML.sanitized(html), "<p>a</p>")
    }

    func testSanitizedRemovesSelfClosingScript() {
        let html = "<p>a</p><script src=\"evil.js\" />"
        XCTAssertEqual(EntryHTML.sanitized(html), "<p>a</p>")
    }

    func testSanitizedRemovesMultiLineScript() {
        let html = """
        <p>a</p>
        <script type="text/javascript">
        var x = 1;
        alert(x);
        </script>
        <p>b</p>
        """
        let sanitized = EntryHTML.sanitized(html)
        XCTAssertFalse(sanitized.contains("alert"))
        XCTAssertFalse(sanitized.lowercased().contains("<script"))
        XCTAssertTrue(sanitized.contains("<p>a</p>"))
        XCTAssertTrue(sanitized.contains("<p>b</p>"))
    }

    func testSanitizedRemovesEventHandlers() {
        let html = "<p onclick=\"alert(1)\" onmouseover='x()' class=\"keep\">text</p>"
        let sanitized = EntryHTML.sanitized(html)
        XCTAssertFalse(sanitized.contains("alert(1)"))
        XCTAssertFalse(sanitized.contains("x()"))
        XCTAssertFalse(sanitized.lowercased().contains("onclick"))
        XCTAssertFalse(sanitized.lowercased().contains("onmouseover"))
        XCTAssertTrue(sanitized.contains("class=\"keep\""))
        XCTAssertTrue(sanitized.contains("text"))
    }

    func testSanitizedKeepsAttributesThatMerelyMentionScript() {
        let html = "<span title=\"script\" data-note=\"use a script here\">text</span>"
        XCTAssertEqual(EntryHTML.sanitized(html), html)
    }

    func testSanitizedKeepsOrdinaryAttributesStartingWithOn() {
        let html = "<div only=\"one\" once=\"twice\">text</div>"
        XCTAssertEqual(EntryHTML.sanitized(html), html)
    }

    func testSanitizedLeavesCommentsAlone() {
        let html = "<!-- <script>alert(1)</script> --><p>a</p>"
        XCTAssertEqual(EntryHTML.sanitized(html), html)
    }

    func testSanitizedKeepsStyleBlocks() {
        let html = "<style>p { color: red; }</style><p>a</p>"
        XCTAssertEqual(EntryHTML.sanitized(html), html)
    }

    func testSanitizedLeavesPlainTextUntouched() {
        let html = "<p>apple — 苹果</p>"
        XCTAssertEqual(EntryHTML.sanitized(html), html)
    }

    // MARK: - entryTarget(from:)

    func testEntryTargetStripsScheme() {
        XCTAssertEqual(EntryHTML.entryTarget(from: URL(string: "entry://apple")!), "apple")
    }

    func testEntryTargetDecodesPercentEscapes() {
        XCTAssertEqual(
            EntryHTML.entryTarget(from: URL(string: "entry://%E8%8B%B9%E6%9E%9C")!),
            "苹果"
        )
    }

    func testEntryTargetDropsTheFragment() {
        XCTAssertEqual(EntryHTML.entryTarget(from: URL(string: "entry://apple#n.")!), "apple")
    }

    func testEntryTargetDropsTheQuery() {
        XCTAssertEqual(EntryHTML.entryTarget(from: URL(string: "entry://apple?x=1")!), "apple")
    }

    func testEntryTargetIsNilWithoutAKey() {
        XCTAssertNil(EntryHTML.entryTarget(from: URL(string: "entry://")!))
    }

    func testEntryTargetIsNilForAnEmptyFragmentOnly() {
        XCTAssertNil(EntryHTML.entryTarget(from: URL(string: "entry://#n.")!))
    }

    func testEntryTargetPreservesCase() {
        XCTAssertEqual(EntryHTML.entryTarget(from: URL(string: "entry://Apple")!), "Apple")
    }

    func testEntryTargetIsNilForOtherSchemes() {
        XCTAssertNil(EntryHTML.entryTarget(from: URL(string: "https://example.com/apple")!))
    }

    // MARK: - rewriteResources(_:)

    func testRelativeImageSourceIsRoutedToTheResourceScheme() {
        let html = "<p><img src=\"pic/apple.png\"></p>"
        XCTAssertTrue(
            EntryHTML.rewriteResources(html)
                .contains("src=\"odc-resource:///pic/apple.png\"")
        )
    }

    func testBackslashPathsAreNormalised() {
        let html = "<img src=\"\\pic\\apple.png\">"
        XCTAssertTrue(
            EntryHTML.rewriteResources(html)
                .contains("src=\"odc-resource:///pic/apple.png\"")
        )
    }

    func testAbsoluteAndEntryLinksAreUntouched() {
        let html = """
        <img src="https://example.com/a.png">
        <a href="entry://apple">apple</a>
        <a href="#top">top</a>
        <img src="data:image/png;base64,AAAA">
        """
        let rewritten = EntryHTML.rewriteResources(html)
        XCTAssertTrue(rewritten.contains("src=\"https://example.com/a.png\""))
        XCTAssertTrue(rewritten.contains("href=\"entry://apple\""))
        XCTAssertTrue(rewritten.contains("href=\"#top\""))
        XCTAssertTrue(rewritten.contains("src=\"data:image/png;base64,AAAA\""))
    }

    func testCssURLsAreRewritten() {
        let html = "<style>.icon { background: url(\\pic\\icon.png); }</style>"
        XCTAssertTrue(
            EntryHTML.rewriteResources(html)
                .contains("url(odc-resource:///pic/icon.png)")
        )
    }

    func testAudioElementsAreRouted() {
        let html = "<audio controls src=\"\\sound\\apple.mp3\"></audio>"
        XCTAssertTrue(
            EntryHTML.rewriteResources(html)
                .contains("src=\"odc-resource:///sound/apple.mp3\"")
        )
    }

    func testHtmlWithoutResourcesIsUnchanged() {
        let html = "<p>apple</p>"
        XCTAssertEqual(EntryHTML.rewriteResources(html), html)
    }

    func testSoundLinksAreRoutedToTheRepository() {
        let html = "<a href=\"sound://hwd/ame/8/memory.mp3\"><img src=\"img/spkr_b.png\"></a>"
        let rewritten = EntryHTML.rewriteResources(html)
        XCTAssertTrue(
            rewritten.contains("href=\"odc-resource:///hwd/ame/8/memory.mp3\""),
            rewritten
        )
        XCTAssertTrue(rewritten.contains("src=\"odc-resource:///img/spkr_b.png\""), rewritten)
    }

    func testDoubleSlashResourcesAreRouted() {
        let html = "<audio src=\"//sound/apple.mp3\"></audio>"
        XCTAssertTrue(
            EntryHTML.rewriteResources(html)
                .contains("src=\"odc-resource:///sound/apple.mp3\"")
        )
    }

    // MARK: - Single page

    private func section(_ html: String, name: String = "Dictionary") -> EntryHTML.PageSection {
        EntryHTML.PageSection(dictionaryID: name, name: name, html: html)
    }

    func testPagePutsEverySectionInADeclarativeShadowRoot() {
        let document = EntryHTML.page([
            section("<p>one</p>", name: "A"),
            section("<p>two</p>", name: "B"),
        ])

        XCTAssertTrue(document.hasPrefix("<!DOCTYPE html>"))
        XCTAssertEqual(
            document.components(separatedBy: "<template shadowrootmode=\"open\">").count - 1,
            2
        )
        XCTAssertEqual(document.components(separatedBy: "class=\"odc-body\"").count - 1, 2)
        XCTAssertFalse(document.contains("<script"), document)
    }

    func testPageShowsDictionaryTitlesOnlyWhenSeveralDictionariesShareThePage() {
        let single = EntryHTML.page([section("<p>x</p>", name: "Only")])
        XCTAssertFalse(single.contains("<h2 class=\"odc-dictionary\""), single)

        let both = EntryHTML.page([
            section("<p>x</p>", name: "First"),
            section("<p>y</p>", name: "Second"),
        ])
        XCTAssertTrue(both.contains("<h2 class=\"odc-dictionary\">First</h2>"), both)
        XCTAssertTrue(both.contains("<h2 class=\"odc-dictionary\">Second</h2>"), both)
    }

    func testPageKeepsTheSectionsInOrder() {
        let document = EntryHTML.page([
            section("<p>alpha</p>", name: "A"),
            section("<p>beta</p>", name: "B"),
            section("<p>gamma</p>", name: "C"),
        ])

        guard let alpha = document.range(of: "alpha")?.lowerBound,
              let beta = document.range(of: "beta")?.lowerBound,
              let gamma = document.range(of: "gamma")?.lowerBound
        else {
            return XCTFail("a section went missing: \(document)")
        }
        XCTAssertTrue(alpha < beta && beta < gamma)
    }

    func testPageCarriesTheDividerStylesheet() {
        let document = EntryHTML.page([section("<p>x</p>")])
        XCTAssertTrue(document.contains(".odc-section + .odc-section"), document)
    }

    func testPageScopesResourcesToTheirSection() {
        let document = EntryHTML.page([
            section("<img src=\"pic.png\">", name: "A"),
            section("<img src=\"pic.png\">", name: "B"),
        ])

        XCTAssertTrue(document.contains("src=\"odc-resource:///s0/pic.png\""), document)
        XCTAssertTrue(document.contains("src=\"odc-resource:///s1/pic.png\""), document)
    }

    func testPageScopesEntryLinksToTheirSection() {
        let document = EntryHTML.page([
            section("<a href=\"entry://word\">link</a>", name: "A"),
            section("<a href=\"entry://word\">link</a>", name: "B"),
        ])

        XCTAssertTrue(document.contains("href=\"entry://s0/word\""), document)
        XCTAssertTrue(document.contains("href=\"entry://s1/word\""), document)
    }

    func testPageRewritesEntryStyleBlocksForTheShadowRoot() {
        let document = EntryHTML.page([
            section("<style>body { color: red; } p { margin: 0; }</style><p>x</p>"),
        ])

        XCTAssertTrue(document.contains(".odc-body { color: red; }"), document)
        XCTAssertFalse(document.contains("<style>body {"), document)
        XCTAssertTrue(document.contains("p { margin: 0; }"), document)
    }

    func testShadowCSSMapsPageSelectorsOntoTheHost() {
        let css = """
        html { color: red; }
        :root { --tone: 1; }
        body.foo a { color: blue; }
        body .inner { margin: 0; }
        @media screen { body { padding: 0; } }
        """
        let scoped = EntryHTML.shadowCSS(css)

        XCTAssertTrue(scoped.contains(":host { color: red; }"), scoped)
        XCTAssertTrue(scoped.contains(":host { --tone: 1; }"), scoped)
        XCTAssertTrue(scoped.contains(".odc-body.foo a { color: blue; }"), scoped)
        XCTAssertTrue(scoped.contains(".odc-body .inner { margin: 0; }"), scoped)
        XCTAssertTrue(scoped.contains("@media screen {"), scoped)
        XCTAssertTrue(scoped.contains(".odc-body { padding: 0; }"), scoped)
    }

    func testShadowCSSLeavesNamesThatOnlyLookLikeElements() {
        let css = ".html-body { color: red; } [data-body] { color: blue; }"
        XCTAssertEqual(EntryHTML.shadowCSS(css), css)
    }

    func testPageDropsScriptsFromEverySection() {
        let document = EntryHTML.page([
            section("<script>alert(1)</script><p>one</p>", name: "A"),
            section("<p onclick=\"x()\">two</p>", name: "B"),
        ])

        XCTAssertFalse(document.contains("<script"), document)
        XCTAssertFalse(document.contains("onclick"), document)
    }

    func testAddressedEntryTargetSplitsTheSection() {
        let addressed = EntryHTML.addressedEntryTarget(from: URL(string: "entry://s2/word")!)
        XCTAssertEqual(addressed?.section, 2)
        XCTAssertEqual(addressed?.target, "word")

        let plain = EntryHTML.addressedEntryTarget(from: URL(string: "entry://word")!)
        XCTAssertNil(plain?.section)
        XCTAssertEqual(plain?.target, "word")
    }

    func testSectionIndexReadsOnlySNumberPrefixes() {
        XCTAssertEqual(EntryHTML.sectionIndex(from: "s0"), 0)
        XCTAssertEqual(EntryHTML.sectionIndex(from: "s12"), 12)
        XCTAssertNil(EntryHTML.sectionIndex(from: "s"))
        XCTAssertNil(EntryHTML.sectionIndex(from: "sound"))
        XCTAssertNil(EntryHTML.sectionIndex(from: "s2a"))
    }

    /// A section with no markup belongs to the parsed layout, not the page.
    func testHasMarkupIgnoresEmptyAndWhitespaceOnlyBodies() {
        XCTAssertFalse(EntryHTML.hasMarkup(nil))
        XCTAssertFalse(EntryHTML.hasMarkup(""))
        XCTAssertFalse(EntryHTML.hasMarkup("   \n  "))
        XCTAssertTrue(EntryHTML.hasMarkup("<p>x</p>"))
    }

    // MARK: - One document, one shadow root per dictionary (v0.5.1)

    /// The page scrolls natively only while it stays a single document: one
    /// doctype, one section and one declarative shadow root per dictionary.
    func testTwoDictionariesShareOneDocumentWithASectionAndShadowRootEach() {
        let document = EntryHTML.page([
            section("<p>alpha</p>", name: "Alpha"),
            section("<p>beta</p>", name: "Beta"),
        ])

        XCTAssertEqual(document.components(separatedBy: "<!DOCTYPE html>").count - 1, 1, document)
        XCTAssertEqual(document.components(separatedBy: "<html>").count - 1, 1, document)
        XCTAssertEqual(
            document.components(separatedBy: "<section class=\"odc-section\">").count - 1,
            2,
            document
        )
        XCTAssertEqual(
            document.components(separatedBy: "<template shadowrootmode=\"open\">").count - 1,
            2,
            document
        )
        XCTAssertTrue(document.contains("<h2 class=\"odc-dictionary\">Alpha</h2>"), document)
        XCTAssertTrue(document.contains("<h2 class=\"odc-dictionary\">Beta</h2>"), document)
        guard let alpha = document.range(of: "Alpha</h2>")?.lowerBound,
              let beta = document.range(of: "Beta</h2>")?.lowerBound
        else { return XCTFail("a dictionary title went missing: \(document)") }
        XCTAssertTrue(alpha < beta, document)
    }

    /// A lone dictionary keeps the page chrome quiet: no title, same
    /// single-document shape.
    func testSingleDictionaryPageHasNoTitleAndOneShadowRoot() {
        let document = EntryHTML.page([section("<p>x</p>", name: "Only")])

        XCTAssertEqual(document.components(separatedBy: "<!DOCTYPE html>").count - 1, 1, document)
        XCTAssertEqual(
            document.components(separatedBy: "<section class=\"odc-section\">").count - 1,
            1,
            document
        )
        XCTAssertEqual(
            document.components(separatedBy: "<template shadowrootmode=\"open\">").count - 1,
            1,
            document
        )
        XCTAssertFalse(document.contains("<h2"), document)
        XCTAssertFalse(document.contains("Only"), document)
    }

    /// A dictionary's own style block must stay inside its shadow template;
    /// the page stylesheet carries no dictionary selectors.
    func testDictionaryStylesStayInsideTheirOwnShadowRoot() {
        let document = EntryHTML.page([
            section("<style>.def { color: red; }</style><p class=\"def\">a</p>", name: "A"),
            section("<style>.def { color: blue; }</style><p class=\"def\">b</p>", name: "B"),
        ])

        guard let firstRoot = document.range(of: "<template shadowrootmode=\"open\">"),
              let secondRoot = document.range(
                  of: "<template shadowrootmode=\"open\">",
                  range: firstRoot.upperBound..<document.endIndex
              )
        else { return XCTFail("expected two shadow templates: \(document)") }

        let pageChrome = document[document.startIndex..<firstRoot.lowerBound]
        let firstSection = document[firstRoot.lowerBound..<secondRoot.lowerBound]
        let secondSection = document[secondRoot.lowerBound...]

        XCTAssertFalse(pageChrome.contains(".def"), "page chrome leaked a dictionary selector: \(pageChrome)")
        XCTAssertTrue(firstSection.contains(".def { color: red; }"), String(firstSection))
        XCTAssertFalse(firstSection.contains("color: blue"), String(firstSection))
        XCTAssertTrue(secondSection.contains(".def { color: blue; }"), String(secondSection))
        XCTAssertFalse(secondSection.contains("color: red"), String(secondSection))
    }

    /// html/body are page elements, not shadow content: the rewrite has to map
    /// them onto the host and .odc-body, and leave at-rule bodies alone except
    /// for the selectors they legitimately contain.
    func testShadowCSSMapsBodyAndHtmlWithoutTouchingAtRulePreludes() {
        XCTAssertEqual(EntryHTML.shadowCSS("body { color: red; }"), ".odc-body { color: red; }")
        XCTAssertEqual(EntryHTML.shadowCSS("html { color: red; }"), ":host { color: red; }")
        XCTAssertEqual(EntryHTML.shadowCSS("body .x { color: red; }"), ".odc-body .x { color: red; }")
        XCTAssertEqual(
            EntryHTML.shadowCSS("@media screen { body { color: red; } }"),
            "@media screen { .odc-body { color: red; } }"
        )
        XCTAssertEqual(
            EntryHTML.shadowCSS("@keyframes fade { from { opacity: 0; } to { opacity: 1; } }"),
            "@keyframes fade { from { opacity: 0; } to { opacity: 1; } }"
        )
    }

    /// The same sheet key resolved for two dictionaries must land in its own
    /// section instead of being inlined into both.
    func testEachSectionCarriesItsOwnInlinedStylesheet() {
        let document = EntryHTML.page([
            EntryHTML.PageSection(
                dictionaryID: "a",
                name: "A",
                html: "<p class=\"def\">one</p>",
                stylesheets: [EntryHTML.LinkedStylesheet(key: "style.css", css: ".def { color: red; }")]
            ),
            EntryHTML.PageSection(
                dictionaryID: "b",
                name: "B",
                html: "<p class=\"def\">two</p>",
                stylesheets: [EntryHTML.LinkedStylesheet(key: "style.css", css: ".def { color: blue; }")]
            ),
        ])

        XCTAssertEqual(document.components(separatedBy: ".def { color: red; }").count - 1, 1, document)
        XCTAssertEqual(document.components(separatedBy: ".def { color: blue; }").count - 1, 1, document)
        guard let red = document.range(of: ".def { color: red; }"),
              let blue = document.range(of: ".def { color: blue; }")
        else { return XCTFail("a stylesheet went missing: \(document)") }
        XCTAssertTrue(red.lowerBound < blue.lowerBound, document)
    }

}
