import Foundation
import UniformTypeIdentifiers
import XCTest
@testable import ODictKit

final class MDXImportTests: XCTestCase {
    func testReadableTypesAcceptMDXOnMacOS() {
        #if os(macOS)
        let extensions = ODictDocument.readableContentTypes.map(\.preferredFilenameExtension)
        XCTAssertTrue(extensions.contains("mdx"), "expected .mdx in \(extensions)")
        #endif
        XCTAssertTrue(ODictDocument.readableContentTypes.contains(ODictDocument.contentType))
    }

    func testReportDecodesCountsAndSummary() throws {
        let json = """
        {
          "imported_entries": 339814,
          "written": 339814,
          "removed": 0,
          "skipped_blocks": 1,
          "skipped_entries": 123,
          "corrupt_blocks": [
            {
              "ordinal": 1601,
              "file_offset": 37510487,
              "compressed_size": 21942,
              "declared_size": 65419,
              "decompressed_size": 0,
              "decompressed_offset": 103558482,
              "detail": "zlib: corrupt deflate stream"
            }
          ],
          "deduplicated": 0,
          "skipped_empty": 0,
          "skipped_oversized": 0,
          "working_set": 339814,
          "scan_warning": null,
          "header": {
            "engine_version": "2.0",
            "required_engine_version": "2.0",
            "encoding": "UTF-8",
            "encrypted": null,
            "records": 339814
          },
          "output_bytes": 221196288,
          "mdd_files": [
            { "path": "/tmp/dictionary.mdd", "entries": 3, "bytes": 4096 },
            { "path": "/tmp/dictionary.1.mdd", "entries": 2, "bytes": 2048 }
          ],
          "resource_count": 5,
          "resource_bytes": 6144,
          "warnings": ["/tmp/dictionary.2.mdd: unexpected end of file"]
        }
        """
        let report = try MDXImportReport.decode(from: Data(json.utf8))
        XCTAssertEqual(report.importedEntries, 339814)
        XCTAssertEqual(report.skippedBlocks, 1)
        XCTAssertEqual(report.skippedEntries, 123)
        XCTAssertEqual(report.corruptBlocks.first?.ordinal, 1601)
        XCTAssertEqual(report.header.engineVersion, "2.0")
        XCTAssertNil(report.scanWarning)
        XCTAssertEqual(report.mddFiles.count, 2)
        XCTAssertEqual(report.mddFiles.first?.entries, 3)
        XCTAssertEqual(report.resourceCount, 5)
        XCTAssertEqual(report.resourceBytes, 6144)
        XCTAssertEqual(report.warnings.count, 1)
        XCTAssertTrue(report.summary.hasPrefix("已导入 339814 条，跳过 1 个损坏块（123 条）"))
        XCTAssertTrue(report.summary.contains("资源 5 个"))
        XCTAssertTrue(report.summary.contains("dictionary.2.mdd"))
    }

    func testSummaryKeepsTheScanWarning() throws {
        let json = """
        {
          "imported_entries": 5, "written": 5, "removed": 0,
          "skipped_blocks": 0, "skipped_entries": 0, "corrupt_blocks": [],
          "deduplicated": 0, "skipped_empty": 0, "skipped_oversized": 0,
          "working_set": 5,
          "scan_warning": "record section is unscannable; reading the file as-is",
          "header": {
            "engine_version": "2.0", "required_engine_version": "2.0",
            "encoding": "UTF-8", "encrypted": null, "records": 5
          },
          "output_bytes": 1024
        }
        """
        let report = try MDXImportReport.decode(from: Data(json.utf8))
        XCTAssertEqual(report.summary, "已导入 5 条，⚠︎ record section is unscannable; reading the file as-is")
    }

    func testInvalidReportIsRejected() {
        XCTAssertThrowsError(try MDXImportReport.decode(from: Data("not json".utf8)))
    }

    func testOutputLandsNextToTheSource() throws {
        let directory = try Fixture.temporaryDirectory()
        let source = directory.appending(path: "汉语大词典.mdx")
        try Data("x".utf8).write(to: source)

        let output = try MDXImporter.outputURL(for: source)
        XCTAssertEqual(output.lastPathComponent, "汉语大词典.odc")
        XCTAssertEqual(output.deletingLastPathComponent(), directory)
    }

    func testExistingOutputsGetANumericSuffix() throws {
        let directory = try Fixture.temporaryDirectory()
        let source = directory.appending(path: "Big.mdx")
        try Data("x".utf8).write(to: source)
        try Data("1".utf8).write(to: directory.appending(path: "Big.odc"))
        try Data("2".utf8).write(to: directory.appending(path: "Big-1.odc"))

        let output = try MDXImporter.outputURL(for: source)
        XCTAssertEqual(output.lastPathComponent, "Big-2.odc")
    }

    func testUnwritableSourceDirectoryFallsBackToDocuments() throws {
        let directory = try Fixture.temporaryDirectory()
        let readOnly = directory.appending(path: "readonly")
        try FileManager.default.createDirectory(at: readOnly, withIntermediateDirectories: true)
        try FileManager.default.setAttributes(
            [.posixPermissions: 0o555],
            ofItemAtPath: readOnly.path
        )
        defer {
            try? FileManager.default.setAttributes(
                [.posixPermissions: 0o755],
                ofItemAtPath: readOnly.path
            )
        }

        let documents = try Fixture.temporaryDirectory()
        let output = try MDXImporter.outputURL(
            for: readOnly.appending(path: "Big.mdx"),
            documents: documents
        )
        XCTAssertEqual(output, documents.appending(path: "Big.odc"))
    }

    func testNonMDXSourceIsRejected() async throws {
        let directory = try Fixture.temporaryDirectory()
        let source = directory.appending(path: "notes.txt")
        do {
            _ = try await MDXImporter.convert(source: source)
            XCTFail("expected the conversion to be rejected")
        } catch MDXImportError.unsupportedSource {
            // Expected.
        }
    }

    // MARK: - MDD volume discovery

    func testVolumesAreDiscoveredInImportOrder() throws {
        let directory = FileManager.default.temporaryDirectory
            .appending(path: "odc-volumes-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let names = [
            "dictionary.10.mdd",
            "dictionary.2.mdd",
            "dictionary.mdd",
            "dictionary.mdx",
            "dictionary.1.mdd",
            "other.mdd",
        ]
        for name in names {
            FileManager.default.createFile(
                atPath: directory.appending(path: name).path,
                contents: Data()
            )
        }
        let source = directory.appending(path: "dictionary.mdx")
        let found = MDXImporter.volumes(for: source).map { $0.lastPathComponent }
        XCTAssertEqual(
            found,
            ["dictionary.mdd", "dictionary.1.mdd", "dictionary.2.mdd", "dictionary.10.mdd"]
        )
    }

    func testVolumeDiscoveryIgnoresOtherDictionaries() throws {
        let directory = FileManager.default.temporaryDirectory
            .appending(path: "odc-volumes-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let names = ["dictionary.mdx", "dictionary-extra.mdd", "dictionary.x.mdd", "other.3.mdd"]
        for name in names {
            FileManager.default.createFile(
                atPath: directory.appending(path: name).path,
                contents: Data()
            )
        }
        let source = directory.appending(path: "dictionary.mdx")
        XCTAssertEqual(MDXImporter.volumes(for: source), [])
    }

    func testProgressTallyWeighsEntriesAndResources() {
        var tally = MDXImportProgressTally()

        let preparing = tally.apply(stage: .preparing, done: 0, total: 100, volumeIndex: 0, volumeCount: 1)
        XCTAssertEqual(preparing.fraction, 0, accuracy: 0.0001)
        XCTAssertEqual(preparing.detail, "扫描到 100 个资源")
        XCTAssertTrue(preparing.isDeterminate)

        let half = tally.apply(stage: .entries, done: 50, total: 100, volumeIndex: 0, volumeCount: 0)
        XCTAssertEqual(half.fraction, 0.25, accuracy: 0.0001, "50 of 200 records")
        XCTAssertEqual(half.title, "导入词条")
        XCTAssertEqual(half.detail, "词条 50 / 100")

        let entriesDone = tally.apply(stage: .entries, done: 100, total: 100, volumeIndex: 0, volumeCount: 0)
        XCTAssertEqual(entriesDone.fraction, 0.5, accuracy: 0.0001)

        let volumes = tally.apply(stage: .resources, done: 100, total: 100, volumeIndex: 2, volumeCount: 2)
        XCTAssertEqual(volumes.fraction, 1, accuracy: 0.0001)
        XCTAssertEqual(volumes.detail, "资源 100 / 100 · 卷 2/2")
        XCTAssertEqual(volumes.percent, 100)
        XCTAssertEqual(volumes.title, "导入资源")
    }

    func testProgressTallyFormatsCountsAndVolumes() {
        var tally = MDXImportProgressTally()
        _ = tally.apply(stage: .preparing, done: 0, total: 182_429, volumeIndex: 0, volumeCount: 2)
        _ = tally.apply(stage: .entries, done: 52_738, total: 52_738, volumeIndex: 0, volumeCount: 0)
        let sample = tally.apply(
            stage: .resources,
            done: 45_000,
            total: 182_429,
            volumeIndex: 2,
            volumeCount: 2
        )
        XCTAssertEqual(sample.detail, "资源 45,000 / 182,429 · 卷 2/2")
        XCTAssertEqual(sample.percent, 42, "97,738 of 235,167 records")
    }

    func testProgressTallyNeverMovesBackwards() {
        var tally = MDXImportProgressTally()
        _ = tally.apply(stage: .entries, done: 0, total: 100, volumeIndex: 0, volumeCount: 0)
        let ahead = tally.apply(stage: .entries, done: 60, total: 100, volumeIndex: 0, volumeCount: 0)
        let behind = tally.apply(stage: .entries, done: 10, total: 100, volumeIndex: 0, volumeCount: 0)
        XCTAssertGreaterThan(ahead.fraction, 0)
        XCTAssertEqual(behind.fraction, ahead.fraction, accuracy: 0.0001)
    }

    func testProgressTallyHandlesEmptyJobs() {
        var tally = MDXImportProgressTally()
        let empty = tally.apply(stage: .preparing, done: 0, total: 0, volumeIndex: 0, volumeCount: 0)
        XCTAssertEqual(empty.fraction, 0)
        XCTAssertFalse(empty.isDeterminate, "an unknown total keeps the spinner")

        let committing = tally.apply(stage: .committing, done: 0, total: 0, volumeIndex: 0, volumeCount: 0)
        XCTAssertEqual(committing.fraction, 1)

        let finished = tally.finish()
        XCTAssertEqual(finished.fraction, 1)
        XCTAssertEqual(finished.percent, 100)
        XCTAssertEqual(finished.stage, .finished)
    }

    func testLegacyPhaseFollowsTheDetailedStage() {
        var tally = MDXImportProgressTally()
        let preparing = tally.apply(stage: .preparing, done: 0, total: 0, volumeIndex: 0, volumeCount: 0)
        XCTAssertEqual(preparing.phase, .preparing)
        let importing = tally.apply(stage: .entries, done: 1, total: 2, volumeIndex: 0, volumeCount: 0)
        XCTAssertEqual(importing.phase, .importing)
        XCTAssertEqual(tally.finish().phase, .finished)
    }
}
