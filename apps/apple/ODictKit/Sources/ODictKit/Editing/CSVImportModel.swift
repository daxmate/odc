import Foundation
import Observation

/// Summary shown after an import run.
public struct CSVImportReport: Equatable, Sendable {
    /// Entries written for keys the dictionary did not already have.
    public let importedEntries: Int
    /// Rows that produced no write (bad rows and skipped duplicates).
    public let skippedRows: Int
    /// Existing entries replaced by the incoming row.
    public let overwrittenEntries: Int
    /// Existing entries merged with the incoming row.
    public let mergedEntries: Int
    public let failedKeys: [String]
    public let issues: [String]

    public init(
        importedEntries: Int,
        skippedRows: Int,
        overwrittenEntries: Int = 0,
        mergedEntries: Int = 0,
        failedKeys: [String],
        issues: [String]
    ) {
        self.importedEntries = importedEntries
        self.skippedRows = skippedRows
        self.overwrittenEntries = overwrittenEntries
        self.mergedEntries = mergedEntries
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
    /// One-line explanation for imported Excel files (for example, that only
    /// the first non-empty worksheet of a multi-sheet workbook was read).
    public private(set) var sourceNote: String?
    /// Encoding the CSV was decoded with, for example `UTF-8 with BOM`.
    /// Empty for XLSX files, which never go through the text decoder.
    public private(set) var encodingLabel: String?
    public var mapping = CSVColumnMapping()
    public private(set) var phase: Phase = .idle
    public private(set) var processedRows = 0
    public private(set) var importedEntries = 0
    public private(set) var skippedRows = 0
    public private(set) var overwrittenEntries = 0
    public private(set) var mergedEntries = 0
    public private(set) var failedKeys: [String] = []
    public private(set) var rowIssues: [CSVRowIssue] = []
    public private(set) var report: CSVImportReport?
    public var alertMessage: String?

    /// Library the import writes into; scopes mapping templates and the
    /// remembered duplicate policy.
    public let libraryKey: String

    public private(set) var templates: [CSVColumnMappingTemplate] = []
    /// Template that filled the current mapping automatically, if any.
    public private(set) var matchedTemplateName: String?
    /// Draft name typed into the "save mapping as template" prompt.
    public var templateNameDraft = ""

    /// Duplicates found before an import starts, shown by the policy sheet.
    public private(set) var duplicateSummary: CSVDuplicateSummary?
    public private(set) var isPolicyPromptPresented = false
    /// Option highlighted in the duplicate policy sheet.
    public var pendingPolicy: CSVDuplicatePolicy = .skip
    /// Whether the chosen policy is saved for this library.
    public var rememberPolicy = false

    @ObservationIgnored private let editing: ODCEditing
    @ObservationIgnored private let templateStore: CSVColumnTemplateStore
    @ObservationIgnored private let policyStore: CSVDuplicatePolicyStore
    @ObservationIgnored private var plan: CSVImportPlan?

    public init(
        editing: ODCEditing,
        libraryKey: String = "",
        templateStore: CSVColumnTemplateStore? = nil,
        policyStore: CSVDuplicatePolicyStore? = nil
    ) {
        self.editing = editing
        self.libraryKey = libraryKey
        self.templateStore = templateStore ?? CSVColumnTemplateStore()
        self.policyStore = policyStore ?? CSVDuplicatePolicyStore()
        templates = self.templateStore.templates(for: libraryKey)
    }

    /// Issues rendered as one line each, kept for callers that show plain text.
    public var issues: [String] { rowIssues.map(\.displayText) }

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
            let parsed: CSVTable
            let note: String?
            let encoding: String?
            if url.pathExtension.lowercased() == "xlsx" {
                let workbook = try XLSXTable(contentsOf: url)
                parsed = workbook.table
                note = workbook.note
                encoding = nil
            } else {
                let decoded = try CSVTextDecoder.decode(Data(contentsOf: url))
                parsed = try CSVTable(parsing: decoded.text)
                note = nil
                encoding = decoded.encodingLabel
            }
            table = parsed
            fileName = url.lastPathComponent
            sourceNote = note
            encodingLabel = encoding
            mapping = CSVColumnMapping.suggested(for: parsed.header)
            matchedTemplateName = nil
            if let template = templateStore.match(header: parsed.header, for: libraryKey) {
                mapping = template.mapping
                matchedTemplateName = template.name
            }
            makePlan()
            phase = .ready
            alertMessage = nil
        } catch {
            table = nil
            fileName = url.lastPathComponent
            sourceNote = nil
            encodingLabel = nil
            matchedTemplateName = nil
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

    // MARK: - Mapping templates

    /// Re-read the templates saved for this library.
    public func refreshTemplates() {
        templates = templateStore.templates(for: libraryKey)
    }

    /// Save the current mapping under `name`. Returns false when it cannot.
    @discardableResult
    public func saveCurrentMappingAsTemplate(named name: String) -> Bool {
        let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else {
            alertMessage = "请输入模板名称。"
            return false
        }
        guard mapping.isValid else {
            alertMessage = "请先选择 key 列再保存模板。"
            return false
        }
        let template = CSVColumnMappingTemplate(name: trimmed, header: header, mapping: mapping)
        templateStore.save(template, for: libraryKey)
        refreshTemplates()
        matchedTemplateName = trimmed
        templateNameDraft = ""
        return true
    }

    /// Apply a saved template to the current table.
    public func applyTemplate(named name: String) {
        guard let template = templates.first(where: { $0.name == name }) else { return }
        mapping = template.mapping
        matchedTemplateName = template.name
        makePlan()
    }

    public func deleteTemplate(named name: String) {
        templateStore.delete(named: name, for: libraryKey)
        refreshTemplates()
        if matchedTemplateName == name {
            matchedTemplateName = nil
        }
    }

    // MARK: - Duplicate policy

    /// Duplicates the current table and library would produce.
    public func duplicateSummaryForCurrentTable() -> CSVDuplicateSummary {
        guard let table else {
            return CSVDuplicateSummary(internalDuplicates: [], internalExtraRows: 0, libraryConflicts: [])
        }
        return CSVDuplicateDetector.summary(table: table, mapping: mapping, editing: editing)
    }

    public func startImport() {
        guard canImport, let plan else {
            alertMessage = "请先选择 CSV 并指定 key 列。"
            return
        }
        resetCounters()

        let summary = duplicateSummaryForCurrentTable()
        if summary.isEmpty {
            beginImport(plan: plan, policy: .overwrite)
            return
        }
        if let stored = policyStore.policy(for: libraryKey) {
            beginImport(plan: plan, policy: stored)
            return
        }

        duplicateSummary = summary
        pendingPolicy = .skip
        rememberPolicy = false
        isPolicyPromptPresented = true
    }

    /// Apply the option picked in the duplicate sheet and start importing.
    public func confirmPolicy() {
        guard isPolicyPromptPresented else { return }
        let policy = pendingPolicy
        if rememberPolicy {
            policyStore.set(policy, for: libraryKey)
        }
        isPolicyPromptPresented = false
        duplicateSummary = nil
        guard let plan else { return }
        beginImport(plan: plan, policy: policy)
    }

    public func cancelPolicyPrompt() {
        isPolicyPromptPresented = false
        duplicateSummary = nil
    }

    // MARK: - Import

    private func beginImport(plan: CSVImportPlan, policy: CSVDuplicatePolicy) {
        let effective: CSVImportPlan
        if let table, let rebuilt = CSVImportPlan.make(
            table: table,
            mapping: mapping,
            duplicatePolicy: policy
        ) {
            effective = rebuilt
        } else {
            effective = plan
        }

        phase = .importing
        rowIssues = effective.issues
        report = nil

        Task { @MainActor [weak self] in
            _ = self?.performImport(plan: effective, policy: policy)
        }
    }

    private func resetCounters() {
        processedRows = 0
        importedEntries = 0
        skippedRows = 0
        overwrittenEntries = 0
        mergedEntries = 0
        failedKeys = []
        rowIssues = []
        report = nil
    }

    /// Synchronous core used by tests and by the task above.
    @discardableResult
    public func performImport(plan: CSVImportPlan, policy: CSVDuplicatePolicy = .overwrite) -> CSVImportReport {
        var imported = 0
        var skipped = plan.issues.count
        var overwritten = 0
        var merged = 0
        var failures: [String] = []
        var issues = plan.issues
        var processed = 0
        let keyName = mapping.key.flatMap { index in
            header.indices.contains(index) ? header[index] : nil
        }

        for group in plan.groups {
            do {
                if let existing = try? editing.entry(for: group.entry.key) {
                    switch policy {
                    case .skip:
                        skipped += group.rowCount
                        issues.append(
                            CSVRowIssue(
                                row: group.sourceRows.first ?? 1,
                                column: keyName,
                                reason: "重复 key「\(group.entry.key)」，库内已存在，已跳过"
                            )
                        )
                    case .overwrite:
                        _ = try editing.upsert(group.entry)
                        overwritten += 1
                    case .merge:
                        _ = try editing.upsert(
                            CSVDuplicateDetector.merged(library: existing, csv: group.entry)
                        )
                        merged += 1
                    }
                } else {
                    _ = try editing.upsert(group.entry)
                    imported += 1
                }
            } catch {
                failures.append(group.entry.key)
                skipped += group.rowCount
                issues.append(
                    CSVRowIssue(row: group.sourceRows.first ?? 1, reason: "写入失败：\(error.localizedDescription)")
                )
            }
            processed += group.rowCount
            processedRows = processed
            importedEntries = imported
            skippedRows = skipped
            overwrittenEntries = overwritten
            mergedEntries = merged
            failedKeys = failures
            rowIssues = issues
        }

        let unaccounted = plan.totalRows - processed - plan.issues.count
        if unaccounted > 0 {
            skipped += unaccounted
        }
        processedRows = plan.totalRows
        importedEntries = imported
        overwrittenEntries = overwritten
        mergedEntries = merged
        failedKeys = failures

        let report = CSVImportReport(
            importedEntries: imported,
            skippedRows: skipped,
            overwrittenEntries: overwritten,
            mergedEntries: merged,
            failedKeys: failures,
            issues: issues.map(\.displayText)
        )
        self.report = report
        self.rowIssues = issues
        self.skippedRows = skipped
        phase = .finished
        return report
    }
}
