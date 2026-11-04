import Foundation

/// Fixtures are committed as plain files so the tests do not need the Rust toolchain.
enum Fixture {
    /// A repository directory containing a three-entry `odc.db`.
    static var demoRepository: URL {
        URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()
            .appending(path: "Fixtures/demo", directoryHint: .isDirectory)
    }

    /// A small MDX dictionary produced by `odc-testkit gen-mdx 2`.
    static var sampleMDX: URL {
        URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()
            .appending(path: "Fixtures/sample.mdx", directoryHint: .notDirectory)
    }

    /// A single-file pack built from `sampleMDX` with the CLI.
    static var samplePack: URL {
        URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()
            .appending(path: "Fixtures/sample.odc", directoryHint: .notDirectory)
    }

    /// A minimal workbook: shared strings, numbers, inline strings and blanks.
    static var sampleXLSX: URL { xlsx("sample.xlsx") }

    /// Three worksheets where the first one is empty.
    static var multiSheetXLSX: URL { xlsx("multi-sheet.xlsx") }

    /// A worksheet whose first row has a cell but no header text.
    static var blankHeaderXLSX: URL { xlsx("blank-header.xlsx") }

    /// A workbook whose only worksheet has no cells.
    static var emptyXLSX: URL { xlsx("empty.xlsx") }

    /// Not a zip archive at all.
    static var brokenXLSX: URL { xlsx("broken.xlsx") }

    private static func xlsx(_ name: String) -> URL {
        URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()
            .appending(path: "Fixtures/\(name)", directoryHint: .notDirectory)
    }

    static func temporaryDirectory() throws -> URL {
        let url = FileManager.default.temporaryDirectory
            .appending(path: "ODictKitTests-\(UUID().uuidString)", directoryHint: .isDirectory)
        try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        return url
    }
}
