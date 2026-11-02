import Foundation
import Observation

public struct CSVImportReport: Equatable, Sendable {
    public let importedEntries: Int
    public let skippedRows: Int
    public let failedKeys: [String]
    public let issues: [String]

    public init(
        importedEntries: Int,
        skippedRows: Int,
        failedKeys: [String],
        issues: [String]
    ) {
        self.importedEntries = importedEntries
        self.skippedRows = skippedRows
        self.failedKeys = failedKeys
        self.issues = issues
    }
}

/// Drives the CSV import sheet: parse, map columns, preview, then write each
/// merged entry to the working set and report progress by row.
@MainActor
@Observable
public final class CSVImportModel {
    public enum Phase: Equatable {
        case idle
        case ready
        case importing
        case finished
    }

    public private(set) var table: CSVTable?
    public private(set) var fileName: String?
    public var mapping = CSVColumnMapping()
    public private(set) var phase: Phase = .idle
    public private(set) var processedRows = 0
    public private(set) var importedEntries = 0
    public private(set) var skippedRows = 0
    public private(set) var failedKeys: [String] = []
    public private(set) var issues: [String] = []
    public private(set) var report: CSVImportReport?
    public var alertMessage: String?

    @ObservationIgnored private let editing: ODCEditing
    @ObservationIgnored private var plan: CSVImportPlan?

    public init(editing: ODCEditing) {
        self.editing = editing
    }

    /// The plan built from the current table and mapping, exposed for tests
    /// and for the preview count.
    public var preparedPlan: CSVImportPlan? { plan }
    public var header: [String] { table?.header ?? [] }
    public var previewRows: [[String]] { Array((table?.rows ?? []).prefix(20)) }
    public var totalRows: Int { plan?.totalRows ?? table?.rows.count ?? 0 }
    public var canImport: Bool {
        phase == .ready && mapping.isValid
    }

    public func load(url: URL) {
        do {
            let text = try String(contentsOf: url, encoding: .utf8)
            let parsed = try CSVTable(parsing: text)
            table = parsed
            fileName = url.lastPathComponent
            mapping = CSVColumnMapping.suggested(for: parsed.header)
            makePlan()
            phase = .ready
            alertMessage = nil
        } catch {
            table = nil
            fileName = url.lastPathComponent
            plan = nil
            phase = .idle
            alertMessage = error.localizedDescription
        }
    }

    /// Rebuild after the mapping changes; returns false when key is unmapped.
    @discardableResult
    public func makePlan() -> Bool {
        guard let table, let plan = CSVImportPlan.make(table: table, mapping: mapping) else {
            self.plan = nil
            return false
        }
        self.plan = plan
        return true
    }

    public func startImport() {
        guard canImport, let plan else {
            alertMessage = "请先选择 CSV 并指定 key 列。"
            return
        }
        phase = .importing
        processedRows = 0
        importedEntries = 0
        skippedRows = 0
        failedKeys = []
        issues = plan.issues.map { "第 \($0.row) 行：\($0.reason)" }
        report = nil

        Task { @MainActor [weak self] in
            await self?.runImport(plan: plan)
        }
    }

    /// Synchronous core used by tests and the task above.
    public func performImport(plan: CSVImportPlan) -> CSVImportReport {
        var imported = 0
        var skipped = plan.issues.count
        var failures: [String] = []
        var messages = plan.issues.map { "第 \($0.row) 行：\($0.reason)" }
        var processed = 0

        for group in plan.groups {
            do {
                _ = try editing.upsert(group.entry)
                imported += 1
            } catch {
                failures.append(group.entry.key)
                skipped += group.rowCount
                messages.append("key \(group.entry.key)：\(error.localizedDescription)")
            }
            processed += group.rowCount
            processedRows = processed
            importedEntries = imported
            skippedRows = skipped
            failedKeys = failures
            issues = messages
        }

        // Every row is either in a group or listed as an issue; only count a
        // gap when the plan reports something unexpected.
        let unaccounted = plan.totalRows - processed - plan.issues.count
        if unaccounted > 0 {
            skipped += unaccounted
        }
        processedRows = plan.totalRows

        let report = CSVImportReport(
            importedEntries: imported,
            skippedRows: skipped,
            failedKeys: failures,
            issues: messages
        )
        self.report = report
        self.skippedRows = skipped
        phase = .finished
        return report
    }

    private func runImport(plan: CSVImportPlan) async {
        var imported = 0
        var skipped = plan.issues.count
        var failures: [String] = []
        var messages = plan.issues.map { "第 \($0.row) 行：\($0.reason)" }
        var processed = 0

        for (index, group) in plan.groups.enumerated() {
            do {
                _ = try editing.upsert(group.entry)
                imported += 1
            } catch {
                failures.append(group.entry.key)
                skipped += group.rowCount
                messages.append("key \(group.entry.key)：\(error.localizedDescription)")
            }
            processed += group.rowCount
            processedRows = processed
            importedEntries = imported
            skippedRows = skipped
            failedKeys = failures
            issues = messages

            if index % 20 == 19 {
                await Task.yield()
            }
        }

        let unaccounted = plan.totalRows - processed - plan.issues.count
        if unaccounted > 0 {
            skipped += unaccounted
        }
        processedRows = plan.totalRows

        let report = CSVImportReport(
            importedEntries: imported,
            skippedRows: skipped,
            failedKeys: failures,
            issues: messages
        )
        self.report = report
        self.skippedRows = skipped
        phase = .finished
    }
}
