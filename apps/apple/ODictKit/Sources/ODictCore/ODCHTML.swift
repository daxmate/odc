import Foundation
import ODCFFI

/// How `OdcHTML.process` treats the body.
public enum HtmlCheckMode: String, Sendable, CaseIterable {
    /// Report problems without changing a byte.
    case check
    /// Repair, sanitise and format the body.
    case format
}

/// One problem found by the HTML processor.
public struct HtmlDiagnostic: Codable, Equatable, Hashable, Sendable {
    /// Stable code such as `html.parse_error` or `html.removed_element`.
    public let code: String
    /// Short human-readable description.
    public let message: String
    /// Optional extra context (parser position, CSS detail, ...).
    public let detail: String?

    public init(code: String, message: String, detail: String? = nil) {
        self.code = code
        self.message = message
        self.detail = detail
    }
}

/// Result of `OdcHTML.process`.
public struct HtmlProcessResult: Codable, Equatable, Sendable {
    /// Body to show or store. `check` returns the input unchanged.
    public let html: String
    /// Whether `html` differs from the input.
    public let changed: Bool
    /// Every diagnostic, in discovery order.
    public let diagnostics: [HtmlDiagnostic]

    public init(html: String, changed: Bool, diagnostics: [HtmlDiagnostic]) {
        self.html = html
        self.changed = changed
        self.diagnostics = diagnostics
    }
}

/// Static binding for the HTML check / repair entry point.
///
/// The C function has no repository handle and no shared state, so this is the
/// one part of the ODC ABI that is not confined to `OdcDictionary`.
public enum OdcHTML {
    /// Run `check` or `format` over one HTML fragment.
    public static func process(
        _ html: String,
        mode: HtmlCheckMode
    ) throws -> HtmlProcessResult {
        var error: UnsafeMutablePointer<CChar>?
        let result = html.withCString { htmlPointer in
            mode.rawValue.withCString { modePointer in
                odc_html_process_json(htmlPointer, modePointer, &error)
            }
        }
        guard let result else {
            throw OdcError.htmlFailed(errorText(error) ?? "无法处理这段 HTML")
        }
        defer { odc_string_free(result) }
        let data = Data(bytes: result, count: strlen(result))
        do {
            return try JSONDecoder().decode(HtmlProcessResult.self, from: data)
        } catch {
            throw OdcError.invalidResponse(String(describing: error))
        }
    }

    private static func errorText(_ pointer: UnsafeMutablePointer<CChar>?) -> String? {
        guard let pointer else { return nil }
        defer { odc_string_free(pointer) }
        return String(cString: pointer)
    }
}
