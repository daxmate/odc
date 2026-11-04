import Foundation

/// The three first-run entry points shown when no dictionary is open.
public enum LibraryStartAction: String, CaseIterable, Sendable {
    case newDictionary
    case sample
    case open

    /// Button label, kept next to the action so both platforms agree.
    public var title: String {
        switch self {
        case .newDictionary: return "新建词典…"
        case .sample: return "试用示例"
        case .open: return "打开词典…"
        }
    }

    public var systemImage: String {
        switch self {
        case .newDictionary: return "plus.rectangle.on.folder"
        case .sample: return "sparkles"
        case .open: return "folder"
        }
    }
}

/// Which start actions the empty state offers.
///
/// The menu is derived from two facts rather than stored: whether a
/// dictionary is open, and whether the library already has entries. Keeping
/// it as a value lets the views stay dumb and the tests cover every branch.
public enum LibraryStartState: Equatable, Sendable {
    /// No dictionary open, library empty: the true first run.
    case firstRun
    /// No dictionary open, but the library already has entries.
    case libraryNotEmpty
    /// A dictionary is open, so the start menu is not shown.
    case dictionaryOpen

    public init(hasOpenDictionary: Bool, libraryIsEmpty: Bool) {
        if hasOpenDictionary {
            self = .dictionaryOpen
        } else if libraryIsEmpty {
            self = .firstRun
        } else {
            self = .libraryNotEmpty
        }
    }

    /// Actions shown for this state. Creating a dictionary stays available
    /// even when the library already has entries, so a reader is never stuck
    /// looking for the "new" entry point.
    public var actions: [LibraryStartAction] {
        switch self {
        case .dictionaryOpen:
            return []
        case .firstRun, .libraryNotEmpty:
            return [.newDictionary, .sample, .open]
        }
    }

    /// Whether the empty state should list recently imported dictionaries.
    public var showsRecentDictionaries: Bool {
        self == .libraryNotEmpty
    }
}
