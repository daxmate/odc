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

    static func temporaryDirectory() throws -> URL {
        let url = FileManager.default.temporaryDirectory
            .appending(path: "ODictKitTests-\(UUID().uuidString)", directoryHint: .isDirectory)
        try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        return url
    }
}
