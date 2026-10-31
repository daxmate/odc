import UniformTypeIdentifiers

/// Document type shared by the two app targets.
public enum ODictDocument {
    /// Exported UTI declared in both Info.plists.
    public static let contentType = UTType(exportedAs: "com.daxmate.odict.dictionary", conformingTo: .data)

    /// What the file importer accepts: a single-file container, a repository
    /// directory produced by the CLI (one that contains `odc.db`), or — on
    /// macOS only — an MDX dictionary the app converts on import.
    public static var readableContentTypes: [UTType] {
        var types: [UTType] = [contentType, .folder]
        #if os(macOS)
        if let mdx = UTType(filenameExtension: "mdx") {
            types.append(mdx)
        }
        #endif
        return types
    }
}
