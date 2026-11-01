import Foundation
import XCTest
@testable import ODictKit

final class ODCResourceTests: XCTestCase {
    // MARK: - Key normalisation

    func testBackslashesBecomeSlashes() {
        XCTAssertEqual(ODCResource.normalizedKey("\\pic\\apple.png"), "pic/apple.png")
        XCTAssertEqual(ODCResource.normalizedKey("/pic/apple.png"), "pic/apple.png")
        XCTAssertEqual(ODCResource.normalizedKey("./pic/apple.png"), "pic/apple.png")
        XCTAssertEqual(ODCResource.normalizedKey("pic/apple.png"), "pic/apple.png")
    }

    // MARK: - URL building

    func testRelativePathsBecomeResourceURLs() {
        XCTAssertEqual(
            ODCResource.url(for: "\\pic\\apple.png")?.absoluteString,
            "odc-resource:///pic/apple.png"
        )
        XCTAssertEqual(
            ODCResource.url(for: "/sound/apple.mp3")?.absoluteString,
            "odc-resource:///sound/apple.mp3"
        )
        XCTAssertEqual(
            ODCResource.url(for: "pic/a b.png")?.absoluteString,
            "odc-resource:///pic/a%20b.png"
        )
    }

    func testAbsoluteAndFragmentReferencesAreLeftAlone() {
        for raw in [
            "https://example.com/a.png",
            "http://example.com",
            "data:image/png;base64,AAAA",
            "mailto:someone@example.com",
            "tel:+123",
            "javascript:alert(1)",
            "entry://apple",
            "odc-resource:///pic/apple.png",
            "#section",
            "",
        ] {
            XCTAssertNil(ODCResource.url(for: raw), raw)
        }
    }

    func testQueryAndFragmentAreDroppedFromTheKey() {
        XCTAssertEqual(
            ODCResource.url(for: "pic/apple.png?v=2#top")?.absoluteString,
            "odc-resource:///pic/apple.png"
        )
    }

    // MARK: - Request parsing

    func testKeysAreReadBackFromRequests() {
        let triple = URL(string: "odc-resource:///pic/apple.png")!
        XCTAssertEqual(ODCResource.key(from: triple), "pic/apple.png")
        let double = URL(string: "odc-resource://pic/apple.png")!
        XCTAssertEqual(ODCResource.key(from: double), "pic/apple.png")
        let encoded = URL(string: "odc-resource:///pic/a%20b.png")!
        XCTAssertEqual(ODCResource.key(from: encoded), "pic/a b.png")
        let foreign = URL(string: "https://example.com/a.png")!
        XCTAssertNil(ODCResource.key(from: foreign))
    }


    func testSoundSchemeAlsoResolves() {
        XCTAssertEqual(
            ODCResource.url(for: "sound://hwd/ame/8/memory.mp3")?.absoluteString,
            "odc-resource:///hwd/ame/8/memory.mp3"
        )
        XCTAssertEqual(
            ODCResource.url(for: "SOUND://sound/apple.mp3")?.absoluteString,
            "odc-resource:///sound/apple.mp3"
        )
    }

    func testAudioKeysAreRecognised() {
        XCTAssertTrue(ODCResource.isAudio("sound/apple.mp3"))
        XCTAssertTrue(ODCResource.isAudio("sound/apple.WAV"))
        XCTAssertTrue(ODCResource.isAudio("sound/apple.ogg"))
        XCTAssertTrue(ODCResource.isAudio("sound/apple.m4a"))
        XCTAssertTrue(ODCResource.isAudio("sound/apple.aac"))
        XCTAssertFalse(ODCResource.isAudio("pic/apple.png"))
        XCTAssertFalse(ODCResource.isAudio("style.css"))
    }

    // MARK: - MIME types

    func testMimeTypesCoverTheMediaDictionariesUse() {
        XCTAssertEqual(ODCResource.mimeType(for: "pic/a.png"), "image/png")
        XCTAssertEqual(ODCResource.mimeType(for: "pic/a.JPG"), "image/jpeg")
        XCTAssertEqual(ODCResource.mimeType(for: "pic/a.jpeg"), "image/jpeg")
        XCTAssertEqual(ODCResource.mimeType(for: "pic/a.gif"), "image/gif")
        XCTAssertEqual(ODCResource.mimeType(for: "pic/a.webp"), "image/webp")
        XCTAssertEqual(ODCResource.mimeType(for: "pic/a.svg"), "image/svg+xml")
        XCTAssertEqual(ODCResource.mimeType(for: "sound/a.mp3"), "audio/mpeg")
        XCTAssertEqual(ODCResource.mimeType(for: "sound/a.wav"), "audio/wav")
        XCTAssertEqual(ODCResource.mimeType(for: "sound/a.ogg"), "audio/ogg")
        XCTAssertEqual(ODCResource.mimeType(for: "sound/a.m4a"), "audio/mp4")
        XCTAssertEqual(ODCResource.mimeType(for: "sound/a.aac"), "audio/aac")
        XCTAssertEqual(ODCResource.mimeType(for: "font/a.woff"), "font/woff")
        XCTAssertEqual(ODCResource.mimeType(for: "font/a.woff2"), "font/woff2")
        XCTAssertEqual(ODCResource.mimeType(for: "font/a.ttf"), "font/ttf")
        XCTAssertEqual(ODCResource.mimeType(for: "style.css"), "text/css")
        XCTAssertEqual(ODCResource.mimeType(for: "script.js"), "text/javascript")
        XCTAssertEqual(
            ODCResource.mimeType(for: "misc/blob.bin"),
            "application/octet-stream"
        )
        XCTAssertEqual(
            ODCResource.mimeType(for: "no-extension"),
            "application/octet-stream"
        )
    }

    // MARK: - Base URL

    func testBaseURLUsesTheResourceScheme() {
        XCTAssertEqual(ODCResource.baseURL.scheme, "odc-resource")
        XCTAssertEqual(ODCResource.baseURL.absoluteString, "odc-resource:///")
    }
}
