import UniformTypeIdentifiers

/// Document type shared by the two app targets.
public enum ODictDocument {
    /// Exported UTI declared in both Info.plists.
    public static let contentType = UTType(exportedAs: "com.daxmate.odict.dictionary", conformingTo: .data)

    /// What the file importer accepts: a single-file container or a repository
    /// directory produced by the CLI (one that contains `odc.db`).
    public static var readableContentTypes: [UTType] {
        [contentType, .folder]
    }
}
