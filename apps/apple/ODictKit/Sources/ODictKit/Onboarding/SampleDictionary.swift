import Foundation

/// Content and writer for the "试用示例" dictionary.
///
/// The entries are deliberately small and fixed: they exercise the reader,
/// the search index and (for `apple`) the HTML rendering path without
/// shipping any third-party dictionary data.
public enum SampleDictionary {
    /// Name shown in the library and the reader.
    public static let displayName = "示例词典"

    /// Message shown after the sample opens.
    public static let notice = "这是示例词典，可随时在词典库中删除。"

    /// Ten English entries with a part of speech and one example each.
    public static let entries: [DictionaryEntry] = [
        DictionaryEntry(
            key: "apple",
            display: "apple",
            phonetic: ["/ˈæp.əl/"],
            defs: [
                Definition(
                    pos: "noun",
                    sense: "a round fruit with red, green or yellow skin",
                    examples: ["She ate an apple after lunch."]
                )
            ],
            rawHTML: "<p><strong>apple</strong> <em>noun</em> — a round fruit with red, green or yellow skin.</p>"
        ),
        DictionaryEntry(
            key: "book",
            display: "book",
            phonetic: ["/bʊk/"],
            defs: [
                Definition(
                    pos: "noun",
                    sense: "a set of printed pages held together in a cover",
                    examples: ["He borrowed a book from the library."]
                )
            ]
        ),
        DictionaryEntry(
            key: "cloud",
            display: "cloud",
            phonetic: ["/klaʊd/"],
            defs: [
                Definition(
                    pos: "noun",
                    sense: "a white or grey mass of water drops in the sky",
                    examples: ["Dark clouds gathered before the storm."]
                )
            ]
        ),
        DictionaryEntry(
            key: "dream",
            display: "dream",
            phonetic: ["/driːm/"],
            defs: [
                Definition(
                    pos: "noun",
                    sense: "a series of images that happen while you sleep",
                    examples: ["I had a strange dream last night."]
                )
            ]
        ),
        DictionaryEntry(
            key: "forest",
            display: "forest",
            phonetic: ["/ˈfɒr.ɪst/"],
            defs: [
                Definition(
                    pos: "noun",
                    sense: "a large area covered with trees",
                    examples: ["We walked through the forest at dawn."]
                )
            ]
        ),
        DictionaryEntry(
            key: "garden",
            display: "garden",
            phonetic: ["/ˈɡɑː.dən/"],
            defs: [
                Definition(
                    pos: "noun",
                    sense: "a piece of land where flowers and vegetables grow",
                    examples: ["She grows tomatoes in her garden."]
                )
            ]
        ),
        DictionaryEntry(
            key: "harbor",
            display: "harbor",
            phonetic: ["/ˈhɑː.bər/"],
            defs: [
                Definition(
                    pos: "noun",
                    sense: "a sheltered place where ships can stay",
                    examples: ["The fishing boats returned to the harbor."]
                )
            ]
        ),
        DictionaryEntry(
            key: "quiet",
            display: "quiet",
            phonetic: ["/ˈkwaɪ.ət/"],
            defs: [
                Definition(
                    pos: "adjective",
                    sense: "making very little noise",
                    examples: ["The library was quiet in the afternoon."]
                )
            ]
        ),
        DictionaryEntry(
            key: "travel",
            display: "travel",
            phonetic: ["/ˈtræv.əl/"],
            defs: [
                Definition(
                    pos: "verb",
                    sense: "to go from one place to another",
                    examples: ["They travel by train every week."]
                )
            ]
        ),
        DictionaryEntry(
            key: "wonder",
            display: "wonder",
            phonetic: ["/ˈwʌn.dər/"],
            defs: [
                Definition(
                    pos: "verb",
                    sense: "to feel curious about something",
                    examples: ["I wonder what she is thinking."]
                )
            ]
        ),
    ]

    /// Write every entry into a freshly created repository and commit them.
    ///
    /// The caller owns `dictionary` and must close it. A failure part-way
    /// leaves the working set dirty, so the caller removes the repository.
    @MainActor
    public static func populate(_ dictionary: OdcDictionary) throws {
        for entry in entries {
            _ = try dictionary.upsert(entry)
        }
        _ = try dictionary.commit(message: "Add sample entries", author: "ODict")
    }
}
