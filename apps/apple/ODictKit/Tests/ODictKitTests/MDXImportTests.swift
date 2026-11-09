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

    func testOutputNeverLandsNextToTheSource() throws {
        let directory = try Fixture.temporaryDirectory()
        let source = directory.appending(path: "汉语大词典.mdx")
        try Data("x".utf8).write(to: source)
        // The source folder is writable; the conversion must still not go there.
        XCTAssertTrue(FileManager.default.isWritableFile(atPath: directory.path))

        let staging = try Fixture.temporaryDirectory()
        let output = try MDXImporter.outputURL(for: source, documents: staging)
        XCTAssertEqual(output.lastPathComponent, "汉语大词典.odc")
        XCTAssertEqual(output.deletingLastPathComponent(), staging)
        let siblings = try FileManager.default.contentsOfDirectory(atPath: directory.path)
        XCTAssertEqual(siblings, ["汉语大词典.mdx"])
    }

    func testDefaultOutputLivesInApplicationSupport() throws {
        let directory = try Fixture.temporaryDirectory()
        let source = directory.appending(path: "汉语大词典.mdx")
        try Data("x".utf8).write(to: source)

        let output = try MDXImporter.outputURL(for: source)
        let support = try XCTUnwrap(
            FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first
        )
        XCTAssertTrue(
            output.standardizedFileURL.path.hasPrefix(support.standardizedFileURL.path + "/"),
            "expected \(output.path) inside \(support.path)"
        )
        XCTAssertTrue(output.path.contains("/ODict/Imports/"), output.path)
        XCTAssertNotEqual(output.deletingLastPathComponent(), directory)
    }

    func testExistingOutputsGetANumericSuffix() throws {
        let directory = try Fixture.temporaryDirectory()
        let staging = try Fixture.temporaryDirectory()
        let source = directory.appending(path: "Big.mdx")
        try Data("x".utf8).write(to: source)
        try Data("1".utf8).write(to: staging.appending(path: "Big.odc"))
        try Data("2".utf8).write(to: staging.appending(path: "Big-1.odc"))

        let output = try MDXImporter.outputURL(for: source, documents: staging)
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
        XCTAssertEqual(volumes.detail, "导入资源 100 / 100 个 · 卷 2/2")
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
        XCTAssertEqual(sample.detail, "导入资源 45,000 / 182,429 个 · 卷 2/2")
        XCTAssertEqual(sample.percent, 42, "97,738 of 235,167 records")
    }

    func testResourcesStageCountsResourcesInsteadOfEntries() {
        var tally = MDXImportProgressTally()
        _ = tally.apply(stage: .preparing, done: 0, total: 4, volumeIndex: 0, volumeCount: 1)
        let entriesDone = tally.apply(
            stage: .entries,
            done: 265_000,
            total: 265_000,
            volumeIndex: 0,
            volumeCount: 0
        )
        XCTAssertEqual(entriesDone.detail, "词条 265,000 / 265,000")

        // Copying the media is the long part of an illustrated dictionary, so
        // the banner has to show it moving instead of the finished entry count.
        let first = tally.apply(stage: .resources, done: 1, total: 4, volumeIndex: 1, volumeCount: 1)
        XCTAssertEqual(first.title, "导入资源")
        XCTAssertEqual(first.detail, "导入资源 1 / 4 个 · 卷 1/1")
        XCTAssertTrue(first.isDeterminate)
        XCTAssertGreaterThanOrEqual(first.fraction, entriesDone.fraction, "fraction stays continuous")

        let last = tally.apply(stage: .resources, done: 4, total: 4, volumeIndex: 1, volumeCount: 1)
        XCTAssertEqual(last.detail, "导入资源 4 / 4 个 · 卷 1/1")
        XCTAssertGreaterThan(last.fraction, first.fraction)
        XCTAssertEqual(last.fraction, 1, accuracy: 0.0001)
    }

    func testCommittingStageIsIndeterminate() {
        let start = Date(timeIntervalSince1970: 1_000)
        var tally = MDXImportProgressTally(now: { start })
        _ = tally.apply(stage: .preparing, done: 0, total: 2_000, volumeIndex: 0, volumeCount: 1)
        _ = tally.apply(stage: .entries, done: 265_000, total: 265_000, volumeIndex: 0, volumeCount: 0)
        let writing = tally.apply(stage: .resources, done: 1_200, total: 2_000, volumeIndex: 1, volumeCount: 1)

        let committing = tally.apply(
            stage: .committing,
            done: 266_200,
            total: 266_200,
            volumeIndex: 0,
            volumeCount: 0
        )
        XCTAssertFalse(committing.isDeterminate, "packing has no samples of its own")
        XCTAssertEqual(committing.fraction, writing.fraction, accuracy: 0.0001, "no fake 100%")
        XCTAssertEqual(committing.title, "写入 / 打包")
        XCTAssertEqual(committing.detail, "正在写入词典包（265,000 条 + 2,000 个资源），请勿退出")
        XCTAssertEqual(committing.committingSince, start)
        XCTAssertEqual(
            committing.elapsedDetail(at: start.addingTimeInterval(12)),
            "正在写入词典包（265,000 条 + 2,000 个资源），请勿退出 · 已用 12 秒"
        )

        // A repeated sample must not restart the clock.
        let repeated = tally.apply(
            stage: .committing,
            done: 266_200,
            total: 266_200,
            volumeIndex: 0,
            volumeCount: 0
        )
        XCTAssertEqual(repeated.committingSince, start)
        XCTAssertEqual(
            repeated.elapsedDetail(at: start.addingTimeInterval(30)),
            "正在写入词典包（265,000 条 + 2,000 个资源），请勿退出 · 已用 30 秒"
        )
        XCTAssertFalse(repeated.isDeterminate)

        let finished = tally.finish()
        XCTAssertEqual(finished.fraction, 1, accuracy: 0.0001)
        XCTAssertTrue(finished.isDeterminate)
        XCTAssertEqual(finished.elapsedDetail(at: start.addingTimeInterval(90)), "")
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
        XCTAssertEqual(committing.fraction, 0, "packing reports nothing, so the bar holds still")
        XCTAssertFalse(committing.isDeterminate)

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

    // MARK: - Profile contract

    private func reportJSON(contract: String?) -> Data {
        let contractField = contract.map { ",\n          \"contract\": \($0),\n          \"p0_failed\": 1,\n          \"p0_failed_samples\": [\"plain\"]" } ?? ""
        let json = """
        {
          "imported_entries": 2,
          "written": 2,
          "removed": 0,
          "skipped_blocks": 0,
          "skipped_entries": 0,
          "corrupt_blocks": [],
          "deduplicated": 0,
          "skipped_empty": 0,
          "skipped_oversized": 0,
          "working_set": 2,
          "scan_warning": null,
          "header": {
            "engine_version": "2.0",
            "required_engine_version": "2.0",
            "encoding": "UTF-8",
            "encrypted": null,
            "records": 2
          },
          "output_bytes": 1024\(contractField)
        }
        """
        return Data(json.utf8)
    }

    func testReportDecodesTheProfileContract() throws {
        let contract = """
        {
            "total": 2, "adapted": 1, "unadapted": 1, "coverage": 0.5,
            "bag_ok": 1, "order_ok": 1, "structure_ok": 1, "clean_ok": 1,
            "equal": 1, "line": "coverage 1/2 (50.000%) | mapped 1/1 four-dim | unadapted 1"
          }
        """
        let report = try MDXImportReport.decode(from: reportJSON(contract: contract))
        let summary = try XCTUnwrap(report.contract)
        XCTAssertEqual(summary.total, 2)
        XCTAssertEqual(summary.adapted, 1)
        XCTAssertEqual(summary.unadapted, 1)
        XCTAssertEqual(summary.coverage, 0.5, accuracy: 0.0001)
        XCTAssertEqual(summary.equal, 1)
        XCTAssertEqual(summary.fourDimensionRate, 1, accuracy: 0.0001)
        XCTAssertEqual(summary.bagOk, 1)
        XCTAssertEqual(report.p0Failed, 1)
        XCTAssertEqual(report.p0FailedSamples, ["plain"])
        XCTAssertTrue(summary.isWarning, "half the entries unadapted is a warning")
    }

    func testReportWithoutAProfileHasNoContract() throws {
        let report = try MDXImportReport.decode(from: reportJSON(contract: nil))
        XCTAssertNil(report.contract)
        XCTAssertEqual(report.p0Failed, 0)
        XCTAssertEqual(report.p0FailedSamples, [])
        XCTAssertEqual(report.summary, "已导入 2 条")
    }

    func testSummaryAppendsTheProfileSegmentOnlyWhenPresent() throws {
        let contract = """
        {
            "total": 2, "adapted": 2, "unadapted": 0, "coverage": 1.0,
            "bag_ok": 2, "order_ok": 2, "structure_ok": 2, "clean_ok": 2,
            "equal": 2, "line": ""
          }
        """
        let report = try MDXImportReport.decode(from: reportJSON(contract: contract))
        XCTAssertEqual(report.summary, "已导入 2 条 · 适配 coverage 100.0% · 四维 100.0% · 未适配 0")
        XCTAssertEqual(report.contract?.isWarning, false)
    }

    func testMalformedProfileFailsWithAReadableError() async throws {
        let directory = FileManager.default.temporaryDirectory
            .appending(path: "odc-profile-\(UUID().uuidString)", directoryHint: .isDirectory)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let source = directory.appending(path: "dictionary.mdx")
        try Data("x".utf8).write(to: source)
        let profile = directory.appending(path: "broken.profile.json")
        try Data("{ not json".utf8).write(to: profile)

        do {
            _ = try await MDXImporter.convert(
                source: source,
                documents: directory,
                profileURL: profile
            )
            XCTFail("expected the malformed profile to be refused")
        } catch let error as MDXImportError {
            guard case .invalidProfile(let detail) = error else {
                return XCTFail("unexpected error: \(error)")
            }
            XCTAssertTrue(detail.contains("broken.profile.json"), detail)
        }
    }

    func testMissingProfileFailsWithAReadableError() async throws {
        let directory = FileManager.default.temporaryDirectory
            .appending(path: "odc-profile-\(UUID().uuidString)", directoryHint: .isDirectory)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let source = directory.appending(path: "dictionary.mdx")
        try Data("x".utf8).write(to: source)
        let missing = directory.appending(path: "absent.profile.json")

        do {
            _ = try await MDXImporter.convert(
                source: source,
                documents: directory,
                profileURL: missing
            )
            XCTFail("expected the missing profile to be refused")
        } catch let error as MDXImportError {
            guard case .invalidProfile(let detail) = error else {
                return XCTFail("unexpected error: \(error)")
            }
            XCTAssertTrue(detail.contains("absent.profile.json"), detail)
        }
    }
}
