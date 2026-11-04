import Foundation
import ODictCore

/// The HTML check / repair step the editor uses.
///
/// Keeping it a protocol lets the editor model run against a fake in unit
/// tests; production work goes through `OdcHTML.process`.
public protocol ODCHTMLProcessing: Sendable {
    func processHTML(_ html: String, mode: HtmlCheckMode) throws -> HtmlProcessResult
}

/// Default processor backed by the ODC C ABI.
public struct ODCHTMLProcessor: ODCHTMLProcessing {
    public init() {}

    public func processHTML(
        _ html: String,
        mode: HtmlCheckMode
    ) throws -> HtmlProcessResult {
        try OdcHTML.process(html, mode: mode)
    }
}
