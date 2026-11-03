import Foundation
import Observation

/// State behind the macOS publish sheet.
///
/// The model owns validation, the keychain lookup and the call into ODCFFI;
/// the view only binds fields and renders the phase.
@MainActor
@Observable
public final class PublishModel {
    /// Where the sheet currently stands.
    public enum Phase: Equatable, Sendable {
        /// Nothing has been attempted yet.
        case idle
        /// A publish is running.
        case working
        /// The release was written; carries the report to display.
        case succeeded(PublishReport)
        /// The attempt failed; carries text to show without closing the form.
        case failed(String)
    }

    /// Repository directory the release is built from.
    public let repoPath: String
    /// Library identifier used for the per-library publish record.
    public let libraryKey: String
    /// Dictionary name written into the manifest.
    public let libraryName: String

    /// Version label, required.
    public var label: String
    /// Free-form release notes, optional.
    public var notes: String = ""
    /// When true the release also carries a base..HEAD patch.
    public var includePatch: Bool = false
    /// Directory the artifacts are written to, chosen by the user.
    public var outDir: URL?

    /// Keypair metadata, or nil when no key is configured yet.
    public private(set) var identity: PublisherIdentity?
    /// Last successful publish of this library.
    public private(set) var lastRecord: PublishRecord?
    /// Current phase.
    public private(set) var phase: Phase = .idle
    /// True while a publish is in flight.
    public private(set) var isPublishing = false

    @ObservationIgnored private let keyStore: PublisherKeyStore
    @ObservationIgnored private let history: PublishHistoryStore

    /// - Parameters:
    ///   - repoPath: directory holding odc.db.
    ///   - libraryKey: stable identifier of the library, used for history.
    ///   - libraryName: name recorded in the manifest.
    ///   - keyStore: where the publisher key lives; injectable for tests.
    ///   - history: last-publish store; injectable for tests.
    ///   - label: starting label, defaulting to today in yyyy.MM.dd form.
    public init(
        repoPath: String,
        libraryKey: String,
        libraryName: String,
        keyStore: PublisherKeyStore = KeychainPublisherKeyStore(),
        history: PublishHistoryStore = PublishHistoryStore(),
        label: String? = nil
    ) {
        self.repoPath = repoPath
        self.libraryKey = libraryKey
        self.libraryName = libraryName
        self.keyStore = keyStore
        self.history = history
        self.label = label ?? Self.defaultLabel()
        reload()
    }

    /// Today as yyyy.MM.dd; the default version label.
    public static func defaultLabel(now: Date = Date(), calendar: Calendar = .current) -> String {
        let components = calendar.dateComponents([.year, .month, .day], from: now)
        let year = components.year ?? 0
        let month = components.month ?? 0
        let day = components.day ?? 0
        return String(format: "%04d.%02d.%02d", year, month, day)
    }

    /// True when a keypair is stored and usable.
    public var hasIdentity: Bool { identity != nil }

    /// True when a previous publish exists, so an incremental patch is
    /// possible.
    public var canIncludePatch: Bool { lastRecord != nil }

    /// Base commit for the incremental patch, or nil when disabled.
    public var patchBase: String? {
        includePatch ? lastRecord?.head : nil
    }

    /// Error text of a failed attempt, for a caption under the form.
    public var errorMessage: String? {
        if case .failed(let message) = phase { return message }
        return nil
    }

    /// Report of a successful attempt, for the result screen.
    public var report: PublishReport? {
        if case .succeeded(let report) = phase { return report }
        return nil
    }

    /// Re-read the keypair and the last publish record.
    public func reload() {
        do {
            identity = try keyStore.load()
        } catch {
            identity = nil
            phase = .failed(error.localizedDescription)
        }
        lastRecord = history.record(for: libraryKey)
        if lastRecord == nil {
            includePatch = false
        }
    }

    /// Clear a previous failure so the form can be edited again.
    public func clearFailure() {
        if case .failed = phase { phase = .idle }
    }

    /// Run the publish. Validation failures never touch the repository.
    public func publish() async {
        guard !isPublishing else { return }

        let trimmedLabel = label.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmedLabel.isEmpty else {
            phase = .failed("版本标签不能为空。")
            return
        }
        guard let outDir else {
            phase = .failed("请选择输出目录。")
            return
        }
        guard hasIdentity else {
            phase = .failed("尚未配置发布者身份，请先生成或导入密钥。")
            return
        }
        let pem: String?
        do {
            pem = try keyStore.privateKeyPEM()
        } catch {
            phase = .failed(error.localizedDescription)
            return
        }
        guard let pem, !pem.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            phase = .failed("尚未配置发布者身份，请先生成或导入密钥。")
            return
        }

        let repoPath = self.repoPath
        let libraryName = self.libraryName
        let trimmedNotes = notes.trimmingCharacters(in: .whitespacesAndNewlines)
        let base = patchBase

        isPublishing = true
        phase = .working
        do {
            let report = try await Task.detached(priority: .userInitiated) {
                try ODCPublishing.publish(
                    repoPath: repoPath,
                    outDir: outDir.path,
                    name: libraryName,
                    label: trimmedLabel,
                    base: base,
                    keyPEM: pem,
                    signer: nil,
                    notes: trimmedNotes.isEmpty ? nil : trimmedNotes
                )
            }.value
            phase = .succeeded(report)
            let record = PublishRecord(head: report.head, label: trimmedLabel)
            history.update(record, for: libraryKey)
            lastRecord = record
        } catch {
            phase = .failed(error.localizedDescription)
        }
        isPublishing = false
    }
}
