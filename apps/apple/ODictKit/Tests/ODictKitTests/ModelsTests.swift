import XCTest
@testable import ODictKit

final class ModelsTests: XCTestCase {
    func testDecodesSearchHits() throws {
        let json = #"[{"key":"apple","display":"Apple"},{"key":"app","display":"app"}]"#
        let hits = try JSONDecoder().decode([SearchHit].self, from: Data(json.utf8))
        XCTAssertEqual(hits.map(\.key), ["apple", "app"])
        XCTAssertEqual(hits.map(\.display), ["Apple", "app"])
    }

    func testDecodesEntryWithEveryField() throws {
        let json = #"""
        {"key":"apple","display":"apple","phonetic":["ˈæp.əl","ˈæpl"],"defs":[{"pos":"noun","sense":"a fruit","examples":["an apple a day"],"labels":["countable"]}],"raw_html":null,"extra":{"source":"fixture","revision":1}}
        """#
        let entry = try JSONDecoder().decode(DictionaryEntry.self, from: Data(json.utf8))
        XCTAssertEqual(entry.key, "apple")
        XCTAssertEqual(entry.phonetic, ["ˈæp.əl", "ˈæpl"])
        XCTAssertEqual(entry.defs.count, 1)
        XCTAssertEqual(entry.defs.first?.pos, "noun")
        XCTAssertEqual(entry.defs.first?.examples, ["an apple a day"])
        XCTAssertEqual(entry.defs.first?.labels, ["countable"])
        XCTAssertNil(entry.rawHTML)
        XCTAssertEqual(entry.extra["source"], .string("fixture"))
        XCTAssertEqual(entry.extra["revision"], .number(1))
    }

    func testDefinitionFillsMissingFields() throws {
        let json = #"{"sense":"only a sense"}"#
        let def = try JSONDecoder().decode(Definition.self, from: Data(json.utf8))
        XCTAssertEqual(def.sense, "only a sense")
        XCTAssertEqual(def.pos, "")
        XCTAssertEqual(def.examples, [])
        XCTAssertEqual(def.labels, [])
    }

    func testEntryFillsMissingOptionalFields() throws {
        let json = #"{"key":"word"}"#
        let entry = try JSONDecoder().decode(DictionaryEntry.self, from: Data(json.utf8))
        XCTAssertEqual(entry.display, "word")
        XCTAssertEqual(entry.phonetic, [])
        XCTAssertEqual(entry.defs, [])
        XCTAssertNil(entry.rawHTML)
        XCTAssertEqual(entry.extra, [:])
    }

    func testJSONValueRoundTrip() throws {
        let json = #"{"null":null,"flag":true,"count":3,"name":"x","list":[1,2],"nested":{"a":"b"}}"#
        let decoded = try JSONDecoder().decode([String: JSONValue].self, from: Data(json.utf8))
        XCTAssertEqual(decoded["null"], .null)
        XCTAssertEqual(decoded["flag"], .bool(true))
        XCTAssertEqual(decoded["count"], .number(3))
        XCTAssertEqual(decoded["name"], .string("x"))
        XCTAssertEqual(decoded["list"], .array([.number(1), .number(2)]))
        XCTAssertEqual(decoded["nested"], .object(["a": .string("b")]))

        let encoded = try JSONEncoder().encode(decoded)
        let again = try JSONDecoder().decode([String: JSONValue].self, from: encoded)
        XCTAssertEqual(again, decoded)
    }

    func testDisplayTextRendersScalarsAndContainers() {
        XCTAssertEqual(JSONValue.null.displayText, "null")
        XCTAssertEqual(JSONValue.bool(false).displayText, "false")
        XCTAssertEqual(JSONValue.number(42).displayText, "42")
        XCTAssertEqual(JSONValue.number(1.5).displayText, "1.5")
        XCTAssertEqual(JSONValue.array([.number(1), .string("a")]).displayText, "[1, a]")
        XCTAssertEqual(JSONValue.object(["b": .number(2), "a": .number(1)]).displayText, "{a: 1, b: 2}")
    }
}
