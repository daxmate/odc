import CoreFoundation
import Foundation

/// Text decoded from a CSV file, together with the encoding that produced it.
public struct CSVDecodedText: Equatable, Sendable {
    /// Encoding the decoder settled on.
    public enum Encoding: String, Equatable, Sendable {
        case utf8
        case utf8WithBOM
        case gb18030
        case utf16LittleEndian
        case utf16BigEndian

        /// Label shown in the import summary.
        public var label: String {
            switch self {
            case .utf8: "UTF-8"
            case .utf8WithBOM: "UTF-8 with BOM"
            case .gb18030: "GB18030"
            case .utf16LittleEndian: "UTF-16 LE"
            case .utf16BigEndian: "UTF-16 BE"
            }
        }
    }

    public let text: String
    public let encoding: Encoding

    public init(text: String, encoding: Encoding) {
        self.text = text
        self.encoding = encoding
    }

    /// Label shown in the import summary, for example `UTF-8 with BOM`.
    public var encodingLabel: String { encoding.label }
}

/// Why a byte buffer could not be turned into text.
public enum CSVTextDecodingError: LocalizedError, Equatable {
    /// No supported encoding (UTF-8, GB18030, UTF-16) accepted the bytes.
    case unrecognizedEncoding

    public var errorDescription: String? {
        switch self {
        case .unrecognizedEncoding:
            "无法识别文件编码。请在 Excel/Numbers 中另存为 UTF-8 CSV 后重试。"
        }
    }
}

/// Picks an encoding for CSV bytes.
///
/// Excel on Chinese systems exports GBK/GB18030 by default and Windows tools
/// often add a UTF-8 or UTF-16 byte order mark, so the order is:
/// BOM sniffing (UTF-8 / UTF-16), strict UTF-8, GB18030, then UTF-16 that
/// carries a BOM. XLSX files never go through this path.
public enum CSVTextDecoder {
    /// GB18030 covers GBK and GB2312, the encodings Excel actually writes.
    static let gb18030: String.Encoding = {
        let cfEncoding = CFStringEncoding(CFStringEncodings.GB_18030_2000.rawValue)
        return String.Encoding(rawValue: CFStringConvertEncodingToNSStringEncoding(cfEncoding))
    }()

    public static func decode(_ data: Data) throws -> CSVDecodedText {
        if data.isEmpty {
            return CSVDecodedText(text: "", encoding: .utf8)
        }

        if data.starts(with: [0xEF, 0xBB, 0xBF]) {
            if let text = String(data: data.dropFirst(3), encoding: .utf8) {
                return CSVDecodedText(text: text, encoding: .utf8WithBOM)
            }
        }
        if data.starts(with: [0xFF, 0xFE]) {
            if let text = String(data: data, encoding: .utf16LittleEndian) {
                return CSVDecodedText(text: stripBOM(from: text), encoding: .utf16LittleEndian)
            }
        }
        if data.starts(with: [0xFE, 0xFF]) {
            if let text = String(data: data, encoding: .utf16BigEndian) {
                return CSVDecodedText(text: stripBOM(from: text), encoding: .utf16BigEndian)
            }
        }
        if let text = String(data: data, encoding: .utf8), isPlausible(text) {
            return CSVDecodedText(text: text, encoding: .utf8)
        }
        if let text = String(data: data, encoding: gb18030), isPlausible(text) {
            return CSVDecodedText(text: text, encoding: .gb18030)
        }
        throw CSVTextDecodingError.unrecognizedEncoding
    }

    /// Drops a leading byte order mark left behind by some decoders.
    private static func stripBOM(from text: String) -> String {
        text.first == "\u{FEFF}" ? String(text.dropFirst()) : text
    }

    /// Rejects mojibake: replacement characters or NUL bytes mean the bytes
    /// were not really text in that encoding.
    private static func isPlausible(_ text: String) -> Bool {
        !text.contains("\u{FFFD}") && !text.contains("\u{0}")
    }
}
