import Foundation
import ODCFFI

/// Errors surfaced by the C ABI wrapper.
public enum OdcError: LocalizedError, Equatable {
    /// `odc_repo_open` refused the path.
    case openFailed(String)
    /// `odc_repo_search_json` failed.
    case searchFailed(String)
    /// `odc_repo_entry_json` failed (including "entry not found").
    case entryFailed(String)
    /// `limit` was outside the range the C ABI accepts.
    case invalidLimit(Int)
    /// The dictionary was already closed.
    case closed
    /// The core returned JSON this client could not decode.
    case invalidResponse(String)

    public var errorDescription: String? {
        switch self {
        case .openFailed(let detail):
            return "无法打开词典：\(detail)"
        case .searchFailed(let detail):
            return "搜索失败：\(detail)"
        case .entryFailed(let detail):
            return "读取词条失败：\(detail)"
        case .invalidLimit(let limit):
            return "搜索数量 \(limit) 超出 1…200 的范围。"
        case .closed:
            return "词典已关闭。"
        case .invalidResponse(let detail):
            return "词典返回了无法解析的数据：\(detail)"
        }
    }
}

/// Owns one OdcRepo pointer and closes it exactly once, when released.
///
/// The C handle is not thread safe, so the box is only touched through
/// OdcDictionary, which is confined to the main actor.
private final class RepoHandle: @unchecked Sendable {
    let pointer: OpaquePointer

    init(pointer: OpaquePointer) {
        self.pointer = pointer
    }

    deinit {
        odc_repo_close(pointer)
    }
}

/// Thin, main-actor-confined wrapper around the ODCFFI C ABI.
///
/// An `OdcRepo` handle is **not** thread safe: the C header states it may be
/// used by one thread at a time. Confining the wrapper to the main actor keeps
/// every call on a single thread.
@MainActor
public final class OdcDictionary {
    private var repo: RepoHandle?
    private let decoder = JSONDecoder()

    /// Open an existing repository directory (one that contains `odc.db`).
    public init(url: URL) throws {
        var error: UnsafeMutablePointer<CChar>?
        let opened = url.path.withCString { path in
            odc_repo_open(path, &error)
        }
        guard let opened else {
            throw OdcError.openFailed(Self.take(error) ?? "无法读取 \(url.lastPathComponent)")
        }
        repo = RepoHandle(pointer: opened)
    }

    public var isOpen: Bool { repo != nil }

    /// Prefix search over normalised keys. `limit` must be within 1…200.
    public func search(_ query: String, limit: Int = 50) throws -> [SearchHit] {
        guard (1...200).contains(limit) else {
            throw OdcError.invalidLimit(limit)
        }
        let handle = try requireHandle()
        var error: UnsafeMutablePointer<CChar>?
        let result = query.withCString { query in
            odc_repo_search_json(handle, query, Int32(limit), &error)
        }
        guard let result else {
            throw OdcError.searchFailed(Self.take(error) ?? "搜索失败")
        }
        defer { odc_string_free(result) }
        let data = Data(bytes: result, count: strlen(result))
        do {
            return try decoder.decode([SearchHit].self, from: data)
        } catch {
            throw OdcError.invalidResponse(String(describing: error))
        }
    }

    /// Exact lookup by normalised key.
    public func entry(for key: String) throws -> DictionaryEntry {
        let handle = try requireHandle()
        var error: UnsafeMutablePointer<CChar>?
        let result = key.withCString { key in
            odc_repo_entry_json(handle, key, &error)
        }
        guard let result else {
            throw OdcError.entryFailed(Self.take(error) ?? "未找到词条：\(key)")
        }
        defer { odc_string_free(result) }
        let data = Data(bytes: result, count: strlen(result))
        do {
            return try decoder.decode(DictionaryEntry.self, from: data)
        } catch {
            throw OdcError.invalidResponse(String(describing: error))
        }
    }

    /// Release the underlying handle. Safe to call more than once.
    public func close() {
        repo = nil
    }

    private func requireHandle() throws -> OpaquePointer {
        guard let repo else { throw OdcError.closed }
        return repo.pointer
    }

    private static func take(_ pointer: UnsafeMutablePointer<CChar>?) -> String? {
        guard let pointer else { return nil }
        defer { odc_string_free(pointer) }
        return String(cString: pointer)
    }
}
