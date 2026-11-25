//! The icons the panel's small layouts mark their readings with: Lucide's
//! (lucide.dev, ISC licence: licenses/Lucide-ISC.txt), as the SVG path data
//! it publishes on its 24-unit grid, read into figures any canvas can draw. The sound
//! outputs' are Fluent UI System Icons' forms (MIT: licenses/FluentUI-System-Icons-MIT.txt) traced in Lucide's line.

/// An icon, by what it stands for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Icon {
    /// The overlay's: a square with a pulse in it (Lucide's square-activity).
    Activity,
    Cpu,
    Gpu,
    Memory,
    Disk,
    Network,
    Game,
    Pin,
    /// A tag (Lucide's tag).
    Tag,
    Close,
    /// A padlock: what is asked for before something goes (a PIN).
    Lock,
    /// Three points joined: what goes to and from devices nearby (Lucide's share-2).
    Share,
    Settings,
    Check,
    Pointer,
    Panel,
    Restore,
    Update,
    Quit,
    Layers,
    Clock,
    ImagePlus,
    SkipBack,
    Play,
    Pause,
    SkipForward,
    Music,
    Volume,
    Muted,
    LayoutGrid,
    Gauge,
    ChevronDown,
    ChevronLeft,
    Maximize,
    Minimize,
    /// What is on the station: a file, a web address; the station itself, and what is copied.
    File,
    Link,
    Inbox,
    Clipboard,
    /// The rest of a thing's choices (a menu); how the station's things show: a list of them, pictures.
    Ellipsis,
    List,
    Image,
    /// What adds one more.
    Add,
    /// A folder (among the station's things).
    Folder,
    /// Something dropped here.
    ArrowDown,
    Search,
    Palette,
    Smartphone,
    /// A computer, as a device nearby that is not a phone (Lucide's monitor).
    Monitor,
    /// A notice of Glance's site (Lucide's megaphone).
    Megaphone,
    PencilRuler,
    /// A note taken off the desktop (Lucide's pin-off), a program off quick launch.
    PinOff,
    /// What is taken away for good, or to the Recycle Bin (Lucide's trash-2).
    Trash,
    /// What is no longer listed (Lucide's eye-off).
    EyeOff,
    /// What a note's format bar puts on the words chosen: bold, slanted, struck through, code.
    Bold,
    Italic,
    Strikethrough,
    Code,
    /// What a note's writing menu does: cut, copy, paste, insert a to-do, a table, a rule, a block of code, make a
    /// heading of a line (one, two, three) or words of it again.
    Scissors,
    Copy,
    ClipboardPaste,
    SquareCheck,
    Table,
    Minus,
    SquareCode,
    Heading1,
    Heading2,
    Heading3,
    Pilcrow,
    /// Words (Lucide's text): what the nearby notice shows for text sent.
    Text,
    /// What a note's writing menu does in a table's cell: a row added above it or below it, a column left of it or right
    /// of it (Lucide's arrow-up-to-line, arrow-down-to-line, arrow-left-to-line, arrow-right-to-line).
    ArrowUpToLine,
    ArrowDownToLine,
    ArrowLeftToLine,
    /// An arrow up beside one down: what puts things in another order (Lucide's arrow-up-down).
    ArrowUpDown,
    /// Either of its two alone, where it is in both: which way they go.
    ArrowUpOfTwo,
    ArrowDownOfTwo,
    ArrowRightToLine,
    /// What a note's writing menu does to a link: it edited, its mark taken off (Lucide's square-pen, unlink).
    SquarePen,
    Unlink,
    /// What sends a message (Lucide's send), and what went with one (paperclip).
    Send,
    Paperclip,
    /// How a report stands: open (Lucide's circle-dot), answered (message-circle), fixed (circle-check), closed
    /// (circle-x).
    /// A sum worked out by search (Lucide's calculator), and a web address (globe).
    Calculator,
    Globe,
    CircleDot,
    MessageCircle,
    CircleCheck,
    CircleX,
    /// What search finds: a program (Lucide's app-window), a tool of the system's (wrench), a note (sticky-note).
    AppWindow,
    Wrench,
    StickyNote,
    /// What opens a thing in its own program (Lucide's external-link).
    ExternalLink,
    /// A to-do's day and time (Lucide's calendar); those done shown (Lucide's chevron-up).
    Calendar,
    ChevronUp,
    ChevronRight,
    /// The pointer over a corner that sizes along the rising diagonal, and along the falling one (Lucide's move-diagonal,
    /// move-diagonal-2).
    MoveDiagonal,
    MoveDiagonal2,
    /// The devices' page: a monitor with a phone before it (Lucide's monitor-smartphone).
    MonitorSmartphone,
    Laptop,
    /// The sound's outputs (Fluent's forms in Lucide's line, see the top): a speaker's box, headphones,
    /// a headset (its microphone's arm), and an output to another box that plays it (a line out, a digital one).
    Speaker,
    Headphones,
    Headset,
    AudioOut,
    /// The wireless devices' (Fluent's forms in Lucide's line, as the outputs'): earbuds, a speaker carried about, a
    /// keyboard, a mouse, a watch, and one of no known kind (Lucide's bluetooth).
    Earbuds,
    PortableSpeaker,
    Keyboard,
    Mouse,
    Watch,
    Bluetooth,
    /// The AI component's: Qiuqiu, a round face looking up and to the right, a spark beside it (drawn for Glance in
    /// Lucide's manner: its lines as wide, its spark filled, see `spots`).
    Ai,
}

impl Icon {
    /// What stands for a device of LocalSend's `kind`: a phone, a laptop, else a monitor.
    pub fn of_device(kind: Option<&str>) -> Icon {
        match kind {
            Some("mobile") => Icon::Smartphone,
            Some("laptop") => Icon::Laptop,
            _ => Icon::Monitor,
        }
    }

    /// Lucide's path data for it ("cpu", "gpu", "memory-stick", "hard-drive",
    /// "arrow-down-up", "gamepad-2", "pin", "x", "settings", "check",
    /// "mouse-pointer-2", "panel-right", "undo-2", "refresh-cw", "power", "layers", "clock", "image-plus", "skip-back", "play", "pause", "skip-forward",
    /// "music", "volume-2", "volume-x" (their speaker an older Lucide's, as lines), "layout-grid", "gauge",
    /// "chevron-down", "briefcase", "maximize", "minimize", "file", "link", "inbox", "clipboard", "ellipsis", "list",
    /// "image", "grip-vertical", "plus", "search", "palette", "smartphone", "pencil-ruler", "pin-off", "trash-2", "eye-off", "bold", "italic", "strikethrough", "code",
    /// "scissors", "copy", "clipboard-paste", "square-check-big", "table", "minus", "square-code", "heading-1", "heading-2",
    /// "heading-3", "pilcrow", "arrow-up-to-line", "arrow-down-to-line", "arrow-left-to-line", "arrow-right-to-line",
    /// "square-pen", "unlink", "send", "paperclip", "circle-dot", "message-circle" (an older Lucide's), "circle-check",
    /// "circle-x", "external-link", "calendar", "chevron-up", "monitor-smartphone", "eye", "laptop", from
    /// lucide-static 1.53.0), its
    /// rectangles and circles written as paths.
    fn data(self) -> &'static [&'static str] {
        match self {
            Icon::Cpu => &[
                "M12 20v2", "M12 2v2", "M17 20v2", "M17 2v2", "M2 12h2", "M2 17h2", "M2 7h2", "M20 12h2", "M20 17h2", "M20 7h2", "M7 20v2", "M7 2v2",
                "M6 4h12a2 2 0 0 1 2 2v12a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2z",
                "M9 8h6a1 1 0 0 1 1 1v6a1 1 0 0 1-1 1H9a1 1 0 0 1-1-1V9a1 1 0 0 1 1-1z",
            ],
            Icon::Gpu => &[
                "M2 17h18a2 2 0 0 0 2-2V7a2 2 0 0 0-2-2H2", "M2 21V3", "M7 17v3a1 1 0 0 0 1 1h5a1 1 0 0 0 1-1v-3",
                "M18 11a2 2 0 1 1-4 0a2 2 0 1 1 4 0z", "M10 11a2 2 0 1 1-4 0a2 2 0 1 1 4 0z",
            ],
            Icon::Memory => &[
                "M12 12v-2", "M12 18v-2", "M16 12v-2", "M16 18v-2", "M2 11h1.5", "M20 18v-2", "M20.5 11H22", "M4 18v-2", "M8 12v-2", "M8 18v-2",
                "M4 6h16a2 2 0 0 1 2 2v6a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2z",
            ],
            Icon::Disk => &[
                "M10 16h.01",
                "M2.212 11.577a2 2 0 0 0-.212.896V18a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-5.527a2 2 0 0 0-.212-.896L18.55 5.11A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.11z",
                "M21.946 12.013H2.054", "M6 16h.01",
            ],
            Icon::Network => &["m3 16 4 4 4-4", "M7 20V4", "m21 8-4-4-4 4", "M17 4v16"],
            Icon::Pin => &[
                "M12 17v5",
                "M9 10.76a2 2 0 0 1-1.11 1.79l-1.78.9A2 2 0 0 0 5 15.24V16a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1v-.76a2 2 0 0 0-1.11-1.79l-1.78-.9A2 2 0 0 1 15 10.76V7a1 1 0 0 1 1-1 2 2 0 0 0 0-4H8a2 2 0 0 0 0 4 1 1 0 0 1 1 1z",
            ],
            Icon::Tag => &["M12.586 2.586A2 2 0 0 0 11.172 2H4a2 2 0 0 0-2 2v7.172a2 2 0 0 0 .586 1.414l8.704 8.704a2.426 2.426 0 0 0 3.42 0l6.58-6.58a2.426 2.426 0 0 0 0-3.42z", "M7.5 7.5h.01"],
            Icon::PinOff => &[
                "M12 17v5",
                "M15 9.34V7a1 1 0 0 1 1-1 2 2 0 0 0 0-4H7.89",
                "m2 2 20 20",
                "M9 9v1.76a2 2 0 0 1-1.11 1.79l-1.78.9A2 2 0 0 0 5 15.24V16a1 1 0 0 0 1 1h11",
            ],
            Icon::Trash => &["M10 11v6", "M14 11v6", "M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6", "M3 6h18", "M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"],
            Icon::EyeOff => &[
                "M10.733 5.076a10.744 10.744 0 0 1 11.205 6.575 1 1 0 0 1 0 .696 10.747 10.747 0 0 1-1.444 2.49",
                "M14.084 14.158a3 3 0 0 1-4.242-4.242",
                "M17.479 17.499a10.75 10.75 0 0 1-15.417-5.151 1 1 0 0 1 0-.696 10.75 10.75 0 0 1 4.446-5.143",
                "m2 2 20 20",
            ],
            Icon::Bold => &["M6 12h9a4 4 0 0 1 0 8H7a1 1 0 0 1-1-1V5a1 1 0 0 1 1-1h7a4 4 0 0 1 0 8"],
            Icon::Italic => &["M19 4H10", "M14 20H5", "M15 4 9 20"],
            Icon::Strikethrough => &["M16 4H9a3 3 0 0 0-2.83 4", "M14 12a4 4 0 0 1 0 8H6", "M4 12h16"],
            Icon::Code => &["m16 18 6-6-6-6", "m8 6-6 6 6 6"],
            Icon::Scissors => &["M9 6a3 3 0 1 1-6 0a3 3 0 1 1 6 0z", "M8.12 8.12 12 12", "M20 4 8.12 15.88", "M9 18a3 3 0 1 1-6 0a3 3 0 1 1 6 0z", "M14.8 14.8 20 20"],
            Icon::Copy => &["M10 8h10a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2H10a2 2 0 0 1-2-2V10a2 2 0 0 1 2-2z", "M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2"],
            Icon::ClipboardPaste => &[
                "M11 14h10", "M16 4h2a2 2 0 0 1 2 2v1.344", "m17 18 4-4-4-4", "M8 4H6a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h12a2 2 0 0 0 1.793-1.113",
                "M9 2h6a1 1 0 0 1 1 1v2a1 1 0 0 1-1 1H9a1 1 0 0 1-1-1V3a1 1 0 0 1 1-1z",
            ],
            Icon::SquareCheck => &["M21 10.656V19a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h12.344", "m9 11 3 3L22 4"],
            Icon::Table => &["M12 3v18", "M5 3h14a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2z", "M3 9h18", "M3 15h18"],
            Icon::Minus => &["M5 12h14"],
            Icon::SquareCode => &["m10 9-3 3 3 3", "m14 15 3-3-3-3", "M5 3h14a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2z"],
            Icon::Heading1 => &["M4 12h8", "M4 18V6", "M12 18V6", "m17 12 3-2v8"],
            Icon::Heading2 => &["M4 12h8", "M4 18V6", "M12 18V6", "M21 18h-4c0-4 4-3 4-6 0-1.5-2-2.5-4-1"],
            Icon::Heading3 => &["M4 12h8", "M4 18V6", "M12 18V6", "M17.5 10.5c1.7-1 3.5 0 3.5 1.5a2 2 0 0 1-2 2", "M17 17.5c2 1.5 4 .3 4-1.5a2 2 0 0 0-2-2"],
            Icon::Pilcrow => &["M13 4v16", "M17 4v16", "M19 4H9.5a4.5 4.5 0 0 0 0 9H13"],
            Icon::Text => &["M15 18H3", "M17 6H3", "M21 12H3"],
            Icon::ArrowUpToLine => &["M5 3h14", "m18 13-6-6-6 6", "M12 7v14"],
            Icon::ArrowDownToLine => &["M12 17V3", "m6 11 6 6 6-6", "M19 21H5"],
            Icon::ArrowLeftToLine => &["M3 19V5", "m13 6-6 6 6 6", "M7 12h14"],
            Icon::ArrowRightToLine => &["M17 12H3", "m11 18 6-6-6-6", "M21 5v14"],
            Icon::SquarePen => &["M12 3H5a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7", "M18.375 2.625a1 1 0 0 1 3 3l-9.013 9.014a2 2 0 0 1-.853.505l-2.873.84a.5.5 0 0 1-.62-.62l.84-2.873a2 2 0 0 1 .506-.852z"],
            Icon::Unlink => &[
                "m18.84 12.25 1.72-1.71h-.02a5.004 5.004 0 0 0-.12-7.07 5.006 5.006 0 0 0-6.95 0l-1.72 1.71",
                "m5.17 11.75-1.71 1.71a5.004 5.004 0 0 0 .12 7.07 5.006 5.006 0 0 0 6.95 0l1.71-1.71",
                "M8 2v3", "M2 8h3", "M16 19v3", "M19 16h3",
            ],
            Icon::Close => &["M18 6 6 18", "m6 6 12 12"],
            Icon::Lock => &["M5 11h14a2 2 0 0 1 2 2v7a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-7a2 2 0 0 1 2-2z", "M7 11V7a5 5 0 0 1 10 0v4"],
            Icon::Share => &["M21 5a3 3 0 1 1-6 0a3 3 0 1 1 6 0z", "M9 12a3 3 0 1 1-6 0a3 3 0 1 1 6 0z", "M21 19a3 3 0 1 1-6 0a3 3 0 1 1 6 0z", "m8.59 13.51 6.83 3.98", "m15.41 6.51-6.82 3.98"],
            Icon::Send => &[
                "M14.536 21.686a.5.5 0 0 0 .937-.024l6.5-19a.496.496 0 0 0-.635-.635l-19 6.5a.5.5 0 0 0-.024.937l7.93 3.18a2 2 0 0 1 1.112 1.11z",
                "m21.854 2.147-10.94 10.939",
            ],
            Icon::Paperclip => &["m16 6-8.414 8.586a2 2 0 0 0 2.829 2.829l8.414-8.586a4 4 0 1 0-5.657-5.657l-8.379 8.551a6 6 0 1 0 8.485 8.485l8.379-8.551"],
            Icon::Calculator => &[
                "M6 2h12a2 2 0 0 1 2 2v16a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2z", "M8 6h8", "M16 14v4", "M16 10h.01", "M12 10h.01", "M8 10h.01", "M12 14h.01", "M8 14h.01", "M12 18h.01", "M8 18h.01",
            ],
            Icon::Globe => &["M12 2a10 10 0 1 0 0 20a10 10 0 1 0 0-20z", "M12 2a14.5 14.5 0 0 0 0 20a14.5 14.5 0 0 0 0-20", "M2 12h20"],
            Icon::CircleDot => &["M12 2a10 10 0 1 0 0 20a10 10 0 1 0 0-20z", "M13 12a1 1 0 1 1-2 0a1 1 0 1 1 2 0z"],
            Icon::MessageCircle => &["M7.9 20A9 9 0 1 0 4 16.1L2 22Z"],
            Icon::CircleCheck => &["M12 2a10 10 0 1 0 0 20a10 10 0 1 0 0-20z", "m9 12 2 2 4-4"],
            Icon::CircleX => &["M12 2a10 10 0 1 0 0 20a10 10 0 1 0 0-20z", "m15 9-6 6", "m9 9 6 6"],
            Icon::ArrowUpDown => &["m21 16-4 4-4-4", "M17 20V4", "m3 8 4-4 4 4", "M7 4v16"],
            Icon::ArrowUpOfTwo => &["m3 8 4-4 4 4", "M7 4v16"],
            Icon::ArrowDownOfTwo => &["m21 16-4 4-4-4", "M17 20V4"],
            Icon::AppWindow => &["M4 4h16a2 2 0 0 1 2 2v12a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2z", "M10 4v4", "M2 8h20", "M6 4v4"],
            Icon::Wrench => &["M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.77-3.77a6 6 0 0 1-7.94 7.94l-6.91 6.91a2.12 2.12 0 0 1-3-3l6.91-6.91a6 6 0 0 1 7.94-7.94l-3.76 3.76z"],
            Icon::StickyNote => &["M16 3H5a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2V8z", "M15 3v4a2 2 0 0 0 2 2h4"],
            Icon::ExternalLink => &["M15 3h6v6", "M10 14 21 3", "M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"],
            Icon::ChevronUp => &["m18 15-6-6-6 6"],
            Icon::ChevronRight => &["m9 18 6-6-6-6"],
            Icon::MoveDiagonal => &["M11 19H5v-6", "M13 5h6v6", "M19 5 5 19"],
            Icon::MoveDiagonal2 => &["M19 13v6h-6", "M5 11V5h6", "m5 5 14 14"],
            Icon::MonitorSmartphone => &["M18 8V6a2 2 0 0 0-2-2H4a2 2 0 0 0-2 2v7a2 2 0 0 0 2 2h8", "M10 19v-3.96 3.15", "M7 19h5", "M18 12h2a2 2 0 0 1 2 2v6a2 2 0 0 1-2 2h-2a2 2 0 0 1-2-2v-6a2 2 0 0 1 2-2z"],
            Icon::Ai => &[AI_FACE, "M10.41 10.23 9.79 13.17", "M14.81 10.23 14.19 13.17"],
            Icon::Speaker => &["M8.4 3h7.2a3 3 0 0 1 3 3v12a3 3 0 0 1-3 3H8.4a3 3 0 0 1-3-3V6a3 3 0 0 1 3-3z", "M15 14.4a3 3 0 1 1-6 0a3 3 0 1 1 6 0z", "M12 7.6h.01"],
            Icon::Headphones => &["M3 19.2V12a9 9 0 0 1 18 0v7.2a1.8 1.8 0 0 1-1.8 1.8h-3v-7.2H21", "M3 13.8h4.8V21H4.8A1.8 1.8 0 0 1 3 19.2"],
            Icon::Headset => &["M5.4 9.6a6.6 6.6 0 0 1 13.2 0V13.2a1.8 1.8 0 0 1-1.8 1.8H15V9.6h3.6", "M5.4 9.6H9V15H5.4", "M5.4 9.6v7.2a3 3 0 0 0 3 3h4.2"],
            Icon::Earbuds => &[
                "M18 3a4 4 0 0 0-4 4v12a2 2 0 0 0 4 0v-8a5 5 0 0 0 3.75-1.693a1 1 0 0 0 .25-.661V5.354a1 1 0 0 0-.25-.661A5 5 0 0 0 18 3",
                "M6 3a4 4 0 0 1 4 4v12a2 2 0 0 1-4 0v-8a5 5 0 0 1-3.75-1.693A1 1 0 0 1 2 8.646V5.354a1 1 0 0 1 .25-.661A5 5 0 0 1 6 3",
            ],
            Icon::PortableSpeaker => &["M7.5 7h9a5 5 0 0 1 0 10h-9a5 5 0 0 1 0-10z", "M10 12a2 2 0 1 1-4 0a2 2 0 1 1 4 0z", "M18 12a2 2 0 1 1-4 0a2 2 0 1 1 4 0z"],
            Icon::Keyboard => &["M5 5h14a2 2 0 0 1 2 2v10.5a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V7a2 2 0 0 1 2-2z", "M6.6 9h.01", "M10.2 9h.01", "M13.8 9h.01", "M17.4 9h.01", "M8.4 12.5h.01", "M12 12.5h.01", "M15.6 12.5h.01", "M7.5 16h9"],
            Icon::Mouse => &["M12 2a7 7 0 0 1 7 7v6a7 7 0 0 1-14 0V9a7 7 0 0 1 7-7z", "M12 6v4"],
            Icon::Watch => &[
                "M9 6.6h6a2.4 2.4 0 0 1 2.4 2.4v6a2.4 2.4 0 0 1-2.4 2.4H9a2.4 2.4 0 0 1-2.4-2.4V9A2.4 2.4 0 0 1 9 6.6z",
                "M7.8 6.6V4.8A1.8 1.8 0 0 1 9.6 3h4.8a1.8 1.8 0 0 1 1.8 1.8v1.8",
                "M7.8 17.4v1.8A1.8 1.8 0 0 0 9.6 21h4.8a1.8 1.8 0 0 0 1.8-1.8v-1.8",
                "M19.2 10.2v3.6",
            ],
            Icon::Bluetooth => &["M7 7l10 10-5 5V2l5 5L7 17"],
            Icon::AudioOut => &["M6 6.5h12a3 3 0 0 1 3 3v5a3 3 0 0 1-3 3H6a3 3 0 0 1-3-3v-5a3 3 0 0 1 3-3z", "M18 12a2.5 2.5 0 1 1-5 0a2.5 2.5 0 1 1 5 0z", "M7.5 9.75h.01", "M7.5 14.25h.01"],
            Icon::Laptop => &["M18 5a2 2 0 0 1 2 2v8.526a2 2 0 0 0 .212.897l1.068 2.127a1 1 0 0 1-.9 1.45H3.62a1 1 0 0 1-.9-1.45l1.068-2.127A2 2 0 0 0 4 15.526V7a2 2 0 0 1 2-2z", "M20.054 15.987H3.946"],
            Icon::Calendar => &["M8 2v4", "M16 2v4", "M5 4h14a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2z", "M3 10h18"],
            Icon::File => &["M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7z", "M14 2v4a2 2 0 0 0 2 2h4"],
            Icon::Link => &["M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71", "M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71"],
            Icon::Inbox => &["M22 12h-6l-2 3h-4l-2-3H2", "M5.45 5.11 2 12v6a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-6l-3.45-6.89A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.11z"],
            Icon::Add => &["M5 12h14", "M12 5v14"],
            Icon::ArrowDown => &["M12 5v14", "m19 12-7 7-7-7"],
            Icon::Folder => &["M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2z"],
            Icon::Search => &["M19 11a8 8 0 1 1-16 0a8 8 0 1 1 16 0z", "m21 21-4.34-4.34"],
            Icon::Palette => &[
                "M12 22a1 1 0 0 1 0-20a10 9 0 0 1 10 9a5 5 0 0 1-5 5h-2.25a1.75 1.75 0 0 0-1.4 2.8l0.3 0.4a1.75 1.75 0 0 1-1.4 2.8z",
                "M13 6.5a0.5 0.5 0 1 0 1 0a0.5 0.5 0 1 0-1 0z",
                "M17 10.5a0.5 0.5 0 1 0 1 0a0.5 0.5 0 1 0-1 0z",
                "M6 12.5a0.5 0.5 0 1 0 1 0a0.5 0.5 0 1 0-1 0z",
                "M8 7.5a0.5 0.5 0 1 0 1 0a0.5 0.5 0 1 0-1 0z",
            ],
            Icon::PencilRuler => &[
                "M13 7 8.7 2.7a2.41 2.41 0 0 0-3.4 0L2.7 5.3a2.41 2.41 0 0 0 0 3.4L7 13",
                "m8 6 2-2",
                "m18 16 2-2",
                "m17 11 4.3 4.3c.94.94.94 2.46 0 3.4l-2.6 2.6c-.94.94-2.46.94-3.4 0L11 17",
                "M21.174 6.812a1 1 0 0 0-3.986-3.987L3.842 16.174a2 2 0 0 0-.5.83l-1.321 4.352a.5.5 0 0 0 .623.622l4.353-1.32a2 2 0 0 0 .83-.497z",
                "m15 5 4 4",
            ],
            Icon::Smartphone => &["M7 2h10a2 2 0 0 1 2 2v16a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2z", "M12 18h0.01"],
            Icon::Monitor => &["M4 3h16a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2z", "M8 21h8", "M12 17v4"],
            Icon::Megaphone => &[
                "M11 6a13 13 0 0 0 8.4-2.8A1 1 0 0 1 21 4v12a1 1 0 0 1-1.6.8A13 13 0 0 0 11 14H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2z",
                "M6 14a12 12 0 0 0 2.4 7.2 2 2 0 0 0 3.2-2.4A8 8 0 0 1 10 14",
                "M8 6v8",
            ],
            Icon::Activity => &["M5 3h14a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2z", "M17 12h-2l-2 5-2-10-2 5H7"],
            Icon::Ellipsis => &["M13 12a1 1 0 1 1-2 0a1 1 0 1 1 2 0z", "M20 12a1 1 0 1 1-2 0a1 1 0 1 1 2 0z", "M6 12a1 1 0 1 1-2 0a1 1 0 1 1 2 0z"],
            Icon::List => &["M3 5h.01", "M3 12h.01", "M3 19h.01", "M8 5h13", "M8 12h13", "M8 19h13"],
            Icon::Image => &["M5 3h14a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2z", "M7 9a2 2 0 1 0 4 0a2 2 0 1 0-4 0z", "m21 15-3.086-3.086a2 2 0 0 0-2.828 0L6 21"],
            Icon::Clipboard => &["M9 2h6a1 1 0 0 1 1 1v2a1 1 0 0 1-1 1H9a1 1 0 0 1-1-1V3a1 1 0 0 1 1-1z", "M16 4h2a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h2"],
            Icon::Check => &["M20 6 9 17l-5-5"],
            Icon::Panel => &["M5 3h14a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2z", "M15 3v18"],
            Icon::Restore => &["M9 14 4 9l5-5", "M4 9h10.5a5.5 5.5 0 0 1 5.5 5.5a5.5 5.5 0 0 1-5.5 5.5H11"],
            Icon::Update => &["M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8", "M21 3v5h-5", "M21 12a9 9 0 0 1-9 9 9.75 9.75 0 0 1-6.74-2.74L3 16", "M8 16H3v5"],
            Icon::Quit => &["M12 2v10", "M18.4 6.6a9 9 0 1 1-12.77.04"],
            Icon::Clock => &["M12 2a10 10 0 1 0 0 20a10 10 0 1 0 0-20z", "M12 6v6l4 2"],
            Icon::Maximize => &["M8 3H5a2 2 0 0 0-2 2v3", "M21 8V5a2 2 0 0 0-2-2h-3", "M3 16v3a2 2 0 0 0 2 2h3", "M16 21h3a2 2 0 0 0 2-2v-3"],
            Icon::Minimize => &["M8 3v3a2 2 0 0 1-2 2H3", "M21 8h-3a2 2 0 0 1-2-2V3", "M3 16h3a2 2 0 0 1 2 2v3", "M16 21v-3a2 2 0 0 1 2-2h3"],
            Icon::SkipBack => &["M19 20L9 12L19 4Z", "M5 19V5"],
            Icon::Play => &["M6 3L20 12L6 21Z"],
            Icon::Pause => &["M7 4h2a1 1 0 0 1 1 1v14a1 1 0 0 1-1 1H7a1 1 0 0 1-1-1V5a1 1 0 0 1 1-1z", "M15 4h2a1 1 0 0 1 1 1v14a1 1 0 0 1-1 1h-2a1 1 0 0 1-1-1V5a1 1 0 0 1 1-1z"],
            Icon::SkipForward => &["M5 4L15 12L5 20Z", "M19 5V19"],
            Icon::Music => &["M9 18V5l12-2v13", "M9 18a3 3 0 1 1-6 0a3 3 0 1 1 6 0z", "M21 16a3 3 0 1 1-6 0a3 3 0 1 1 6 0z"],
            Icon::Volume => &["M11 5L6 9H2v6h4l5 4V5Z", "M15.5 8.5a5 5 0 0 1 0 7", "M19 5a10 10 0 0 1 0 14"],
            Icon::Muted => &["M11 5L6 9H2v6h4l5 4V5Z", "M22 9L16 15", "M16 9L22 15"],
            Icon::LayoutGrid => &[
                "M4 3h5a1 1 0 0 1 1 1v5a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1z",
                "M15 3h5a1 1 0 0 1 1 1v5a1 1 0 0 1-1 1h-5a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1z",
                "M15 14h5a1 1 0 0 1 1 1v5a1 1 0 0 1-1 1h-5a1 1 0 0 1-1-1v-5a1 1 0 0 1 1-1z",
                "M4 14h5a1 1 0 0 1 1 1v5a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1v-5a1 1 0 0 1 1-1z",
            ],
            Icon::Gauge => &["m12 14 4-4", "M3.34 19a10 10 0 1 1 17.32 0"],
            Icon::ChevronDown => &["m6 9 6 6 6-6"],
            Icon::ChevronLeft => &["m15 18-6-6 6-6"],
            Icon::ImagePlus => &[
                "M16 5h6",
                "M19 2v6",
                "M21 11.5V19a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h7.5",
                "m21 15-3.086-3.086a2 2 0 0 0-2.828 0L6 21",
                "M7 9a2 2 0 1 0 4 0a2 2 0 1 0-4 0z",
            ],
            Icon::Layers => &[
                "M12.83 2.18a2 2 0 0 0-1.66 0L2.6 6.08a1 1 0 0 0 0 1.83l8.58 3.91a2 2 0 0 0 1.66 0l8.58-3.9a1 1 0 0 0 0-1.83z",
                "M2 12a1 1 0 0 0 .58.91l8.6 3.91a2 2 0 0 0 1.65 0l8.58-3.9A1 1 0 0 0 22 12",
                "M2 17a1 1 0 0 0 .58.91l8.6 3.91a2 2 0 0 0 1.65 0l8.58-3.9A1 1 0 0 0 22 17",
            ],
            Icon::Pointer => &["M4.037 4.688a.495.495 0 0 1 .651-.651l16 6.5a.5.5 0 0 1-.063.947l-6.124 1.58a2 2 0 0 0-1.438 1.435l-1.579 6.126a.5.5 0 0 1-.947.063z"],
            Icon::Settings => &[
                "M9.671 4.136a2.34 2.34 0 0 1 4.659 0 2.34 2.34 0 0 0 3.319 1.915 2.34 2.34 0 0 1 2.33 4.033 2.34 2.34 0 0 0 0 3.831 2.34 2.34 0 0 1-2.33 4.033 2.34 2.34 0 0 0-3.319 1.915 2.34 2.34 0 0 1-4.659 0 2.34 2.34 0 0 0-3.32-1.915 2.34 2.34 0 0 1-2.33-4.033 2.34 2.34 0 0 0 0-3.831A2.34 2.34 0 0 1 6.35 6.051a2.34 2.34 0 0 0 3.319-1.915",
                "M15 12a3 3 0 1 1-6 0a3 3 0 1 1 6 0z",
            ],
            Icon::Game => &[
                "M6 11h4", "M8 9v4", "M15 12h.01", "M18 10h.01",
                "M17.32 5H6.68a4 4 0 0 0-3.978 3.59c-.006.052-.01.101-.017.152C2.604 9.416 2 14.456 2 16a3 3 0 0 0 3 3c1 0 1.5-.5 2-1l1.414-1.414A2 2 0 0 1 9.828 16h4.344a2 2 0 0 1 1.414.586L17 18c.5.5 1 1 2 1a3 3 0 0 0 3-3c0-1.545-.604-6.584-.685-7.258-.007-.05-.011-.1-.017-.151A4 4 0 0 0 17.32 5z",
            ],
        }
    }

    /// Its figures, on the 24-unit grid.
    pub fn figures(self) -> Vec<Figure> {
        self.data().iter().flat_map(|path| parse(path)).collect()
    }

    /// What of it is filled, not only stroked (its edge drawn at half the stroke, see `SPOT_EDGE`): path data, on the
    /// 24-unit grid. None for most.
    fn spot_data(self) -> &'static [&'static str] {
        match self {
            // (Qiuqiu's spark, beside its face at the top right.)
            Icon::Ai => &[AI_SPARK],
            _ => &[],
        }
    }

    /// The figures of what of it is filled (see `spot_data`), on the 24-unit grid.
    pub fn spots(self) -> Vec<Figure> {
        self.spot_data().iter().flat_map(|path| parse(path)).collect()
    }
}

/// How an icon is stroked: this wide on its 24-unit grid, with round ends
/// and joins.
pub const STROKE: f32 = 2.0;

/// Qiuqiu's (the AI icon's) face, and its spark (see `Icon::spots`), about `AI_SPARK_AT`.
const AI_FACE: &str = "M19.3 13a8.3 8.3 0 1 1-16.6 0a8.3 8.3 0 1 1 16.6 0z";
const AI_SPARK: &str = "M20 1.4Q20.42 3.58 22.6 4 20.42 4.42 20 6.6 19.58 4.42 17.4 4 19.58 3.58 20 1.4Z";
const AI_SPARK_AT: (f32, f32) = (20.0, 4.0);

/// Its eyes (each a line from one end to the other): open, closing (a point), shut (a short line across).
const EYES_OPEN: [[f32; 4]; 2] = [[10.41, 10.23, 9.79, 13.17], [14.81, 10.23, 14.19, 13.17]];
const EYES_CLOSING: [[f32; 4]; 2] = [[10.1, 11.7, 10.1, 11.7], [14.5, 11.7, 14.5, 11.7]];
const EYES_SHUT: [[f32; 4]; 2] = [[9.2, 11.55, 11.0, 11.85], [13.6, 11.55, 15.4, 11.85]];

/// How long Qiuqiu greets the pointer come onto it (see `ai_waving`).
pub const AI_WAVE: std::time::Duration = std::time::Duration::from_millis(760);

/// The AI icon `ms` into its greeting (`AI_WAVE` long, as it is at rest at either end): its lines and what of it is
/// filled (see `Icon::spots`), on the 24-unit grid. Its spark shrinks to under a third (300 ms, easing in and out) and
/// springs back past its size (460 ms), and as it does the eyes blink (280 ms from 140 ms: closing to a point, shut a
/// moment, open again).
pub fn ai_waving(ms: f32) -> (Vec<Figure>, Vec<Figure>) {
    let blink = [(140.0, EYES_OPEN), (210.0, EYES_CLOSING), (245.0, EYES_SHUT), (315.0, EYES_SHUT), (350.0, EYES_CLOSING), (420.0, EYES_OPEN)];
    let eyes = match blink.windows(2).find(|pair| ms < pair[1].0) {
        Some([(from, a), (to, b)]) if ms > *from => {
            let k = (ms - from) / (to - from);
            std::array::from_fn(|eye| std::array::from_fn(|i| a[eye][i] + (b[eye][i] - a[eye][i]) * k))
        }
        _ => EYES_OPEN,
    };
    let spark = if ms <= 300.0 {
        1.0 - 0.7 * eased((0.4, 0.0, 0.6, 1.0), (ms / 300.0).max(0.0))
    } else {
        0.3 + 0.7 * eased((0.3, 0.0, 0.2, 1.6), ((ms - 300.0) / 460.0).min(1.0))
    };
    let mut lines = parse(AI_FACE);
    for [x0, y0, x1, y1] in eyes {
        lines.push(Figure { start: (x0, y0), segments: vec![Segment::Line((x1, y1))], closed: false });
    }
    (lines, parse(AI_SPARK).iter().map(|figure| scaled(figure, AI_SPARK_AT, spark)).collect())
}

/// `figure` made `k` times as large about `about`.
fn scaled(figure: &Figure, about: (f32, f32), k: f32) -> Figure {
    let point = |(x, y): (f32, f32)| (about.0 + (x - about.0) * k, about.1 + (y - about.1) * k);
    let segments = figure
        .segments
        .iter()
        .map(|segment| match *segment {
            Segment::Line(to) => Segment::Line(point(to)),
            Segment::Arc { to, radii, rotation, large, clockwise } => Segment::Arc { to: point(to), radii: (radii.0 * k, radii.1 * k), rotation, large, clockwise },
            Segment::Cubic(c1, c2, to) => Segment::Cubic(point(c1), point(c2), point(to)),
        })
        .collect();
    Figure { start: point(figure.start), segments, closed: figure.closed }
}

/// How far along a CSS cubic-bezier ease with control points (`x1`, `y1`) and (`x2`, `y2`) is at `x` (0 to 1) of its
/// time: found by halving on the curve's parameter (its time rises with it where `x1` and `x2` are within 0 to 1).
fn eased((x1, y1, x2, y2): (f32, f32, f32, f32), x: f32) -> f32 {
    let at = |a: f32, b: f32, t: f32| 3.0 * a * t * (1.0 - t) * (1.0 - t) + 3.0 * b * t * t * (1.0 - t) + t * t * t;
    let (mut low, mut high) = (0.0f32, 1.0f32);
    for _ in 0..24 {
        let t = (low + high) / 2.0;
        if at(x1, x2, t) < x {
            low = t;
        } else {
            high = t;
        }
    }
    at(y1, y2, (low + high) / 2.0)
}

/// How wide the edge of what of an icon is filled is drawn (see `Icon::spots`), round as its strokes are.
pub const SPOT_EDGE: f32 = 1.0;

/// A run of segments from a starting point, closed back to it or not.
#[derive(Clone, Debug, PartialEq)]
pub struct Figure {
    pub start: (f32, f32),
    pub segments: Vec<Segment>,
    pub closed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Segment {
    Line((f32, f32)),
    /// An elliptical arc to `to`, as SVG's: its radii, the ellipse's
    /// rotation (degrees), whether it takes the larger arc, and whether it
    /// turns clockwise (in a y-down space).
    Arc { to: (f32, f32), radii: (f32, f32), rotation: f32, large: bool, clockwise: bool },
    Cubic((f32, f32), (f32, f32), (f32, f32)),
}

/// SVG path data read into figures: the commands M, L, H, V, A, C, S, Q, T and Z, each absolute or relative (lower
/// case), a command's arguments repeated for as many segments as they make. (Quadratic curves are made the cubic ones
/// they are.)
pub fn parse(data: &str) -> Vec<Figure> {
    let mut tokens = Tokens { rest: data };
    let mut figures: Vec<Figure> = Vec::new();
    let mut at = (0.0f32, 0.0f32);
    let mut start = at;
    let mut command = None;
    // The last segment's control point nearest its end, a cubic's or a quadratic's (what S and T reflect: after
    // anything else, they take none, the point they start at).
    let mut cubic_control: Option<(f32, f32)> = None;
    let mut quad_control: Option<(f32, f32)> = None;
    while let Some(next) = tokens.command().or(command) {
        let relative = next.is_ascii_lowercase();
        let base = if relative { at } else { (0.0, 0.0) };
        let point = |tokens: &mut Tokens| Some((base.0 + tokens.number()?, base.1 + tokens.number()?));
        let reflected = |control: Option<(f32, f32)>| control.map_or(at, |(x, y)| (2.0 * at.0 - x, 2.0 * at.1 - y));
        let (was_cubic, was_quad) = (cubic_control.take(), quad_control.take());
        match next.to_ascii_uppercase() {
            'M' => {
                let Some(to) = point(&mut tokens) else { break };
                at = to;
                start = to;
                figures.push(Figure { start: to, segments: Vec::new(), closed: false });
                // Pairs after a move are lines.
                command = Some(if relative { 'l' } else { 'L' });
                continue;
            }
            'L' => {
                let Some(to) = point(&mut tokens) else { break };
                at = to;
                push(&mut figures, start, Segment::Line(to));
            }
            'H' => {
                let Some(x) = tokens.number() else { break };
                at = (base.0 + x, at.1);
                push(&mut figures, start, Segment::Line(at));
            }
            'V' => {
                let Some(y) = tokens.number() else { break };
                at = (at.0, base.1 + y);
                push(&mut figures, start, Segment::Line(at));
            }
            'A' => {
                let (Some(rx), Some(ry), Some(rotation), Some(large), Some(clockwise)) = (tokens.number(), tokens.number(), tokens.number(), tokens.flag(), tokens.flag()) else { break };
                let Some(to) = point(&mut tokens) else { break };
                at = to;
                push(&mut figures, start, Segment::Arc { to, radii: (rx, ry), rotation, large, clockwise });
            }
            'C' => {
                let (Some(c1), Some(c2), Some(to)) = (point(&mut tokens), point(&mut tokens), point(&mut tokens)) else { break };
                at = to;
                cubic_control = Some(c2);
                push(&mut figures, start, Segment::Cubic(c1, c2, to));
            }
            'S' => {
                let (Some(c2), Some(to)) = (point(&mut tokens), point(&mut tokens)) else { break };
                let c1 = reflected(was_cubic);
                at = to;
                cubic_control = Some(c2);
                push(&mut figures, start, Segment::Cubic(c1, c2, to));
            }
            'Q' | 'T' => {
                let control = if next.eq_ignore_ascii_case(&'Q') { point(&mut tokens) } else { Some(reflected(was_quad)) };
                let (Some(control), Some(to)) = (control, point(&mut tokens)) else { break };
                let from = at;
                let towards = |end: (f32, f32)| (end.0 + 2.0 / 3.0 * (control.0 - end.0), end.1 + 2.0 / 3.0 * (control.1 - end.1));
                at = to;
                quad_control = Some(control);
                push(&mut figures, start, Segment::Cubic(towards(from), towards(to), to));
            }
            'Z' => {
                if let Some(figure) = figures.last_mut() {
                    figure.closed = true;
                }
                at = start;
                command = None;
                continue;
            }
            _ => break,
        }
        command = Some(next);
    }
    figures
}

/// Adds a segment to the figure under way, or begins one where the last
/// ended (a segment after a close, with no move).
fn push(figures: &mut Vec<Figure>, start: (f32, f32), segment: Segment) {
    match figures.last_mut().filter(|figure| !figure.closed) {
        Some(figure) => figure.segments.push(segment),
        None => figures.push(Figure { start, segments: vec![segment], closed: false }),
    }
}

/// SVG path data, read a command letter, a number or a flag at a time.
struct Tokens<'a> {
    rest: &'a str,
}

impl Tokens<'_> {
    fn skip(&mut self) {
        self.rest = self.rest.trim_start_matches(|c: char| c.is_whitespace() || c == ',');
    }

    fn command(&mut self) -> Option<char> {
        self.skip();
        let c = self.rest.chars().next().filter(char::is_ascii_alphabetic)?;
        self.rest = &self.rest[1..];
        Some(c)
    }

    /// A number: a sign, digits, one point at most, an exponent; another
    /// number may follow at once (".5.5" is two, "2-2" is two).
    fn number(&mut self) -> Option<f32> {
        self.skip();
        let bytes = self.rest.as_bytes();
        let mut end = 0;
        if matches!(bytes.first(), Some(b'+' | b'-')) {
            end = 1;
        }
        let mut point = false;
        while let Some(&b) = bytes.get(end) {
            match b {
                b'0'..=b'9' => end += 1,
                b'.' if !point => {
                    point = true;
                    end += 1
                }
                b'e' | b'E' if end > 0 => {
                    end += 1;
                    if matches!(bytes.get(end), Some(b'+' | b'-')) {
                        end += 1;
                    }
                }
                _ => break,
            }
        }
        let value = self.rest[..end].parse().ok()?;
        self.rest = &self.rest[end..];
        Some(value)
    }

    /// An arc's flag: one digit, 0 or 1, which nothing need separate from
    /// what follows.
    fn flag(&mut self) -> Option<bool> {
        self.skip();
        let flag = match self.rest.as_bytes().first()? {
            b'0' => false,
            b'1' => true,
            _ => return None,
        };
        self.rest = &self.rest[1..];
        Some(flag)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_moves_lines_and_closes() {
        let figures = parse("M6 4h12v2H6z M2 12h2");
        assert_eq!(figures.len(), 2);
        assert_eq!(figures[0].start, (6.0, 4.0));
        assert_eq!(figures[0].segments, vec![Segment::Line((18.0, 4.0)), Segment::Line((18.0, 6.0)), Segment::Line((6.0, 6.0))]);
        assert!(figures[0].closed);
        assert_eq!(figures[1].segments, vec![Segment::Line((4.0, 12.0))]);
        assert!(!figures[1].closed);
    }

    #[test]
    fn reads_relative_commands_and_packed_numbers() {
        // After a relative move, pairs are relative lines; "-7 7-7-7" is
        // four numbers.
        let figures = parse("m19 12-7 7-7-7");
        assert_eq!(figures[0].start, (19.0, 12.0));
        assert_eq!(figures[0].segments, vec![Segment::Line((12.0, 19.0)), Segment::Line((5.0, 12.0))]);
        assert_eq!(parse("M10 16h.01")[0].segments, vec![Segment::Line((10.01, 16.0))]);
    }

    #[test]
    fn reads_arcs() {
        let figures = parse("M14 4a2 2 0 0 1-4 0");
        assert_eq!(figures[0].segments, vec![Segment::Arc { to: (10.0, 4.0), radii: (2.0, 2.0), rotation: 0.0, large: false, clockwise: true }]);
        // Flags packed against what follows.
        let packed = parse("M14 4a4 4 0 1 1-4 0");
        assert!(matches!(packed[0].segments[0], Segment::Arc { large: true, clockwise: true, .. }));
    }

    #[test]
    fn reads_smooth_and_quadratic_curves() {
        // A smooth curve's first control point the last one's reflected about where it starts.
        let smooth = parse("M0 0C1 2 3 2 4 0s3-2 4 0");
        assert_eq!(smooth[0].segments[1], Segment::Cubic((5.0, -2.0), (7.0, -2.0), (8.0, 0.0)));
        // After anything but a cubic, the point it starts at.
        assert_eq!(parse("M0 0L4 0S6 2 8 0")[0].segments[1], Segment::Cubic((4.0, 0.0), (6.0, 2.0), (8.0, 0.0)));
        // A quadratic curve made the cubic it is, and a smooth one after it reflecting its control point.
        let quad = parse("M0 0Q3 6 6 0t6 0");
        assert_eq!(quad[0].segments[0], Segment::Cubic((2.0, 4.0), (4.0, 4.0), (6.0, 0.0)));
        assert_eq!(quad[0].segments[1], Segment::Cubic((8.0, -4.0), (10.0, -4.0), (12.0, 0.0)));
    }

    /// Qiuqiu greets the pointer from its icon at rest and back to it (no jump as it starts or ends), its eyes shut
    /// across in the middle of the blink, its spark shrunk to 0.3 at 300 ms and sprung past its size on its way back.
    #[test]
    fn qiuqiu_greets_from_rest_to_rest_blinking_as_its_spark_springs() {
        let rest = (Icon::Ai.figures(), Icon::Ai.spots());
        let near = |a: &[Figure], b: &[Figure]| a.len() == b.len() && a.iter().zip(b).all(|(a, b)| (a.start.0 - b.start.0).abs() < 1e-3 && (a.start.1 - b.start.1).abs() < 1e-3 && a.segments.len() == b.segments.len());
        for ms in [0.0, AI_WAVE.as_millis() as f32] {
            let (lines, spots) = ai_waving(ms);
            assert!(near(&lines, &rest.0) && near(&spots, &rest.1), "{ms}");
            assert_eq!(lines[1].segments, rest.0[1].segments, "{ms}");
        }
        // (Shut: each eye a short line across.)
        let (lines, _) = ai_waving(280.0);
        assert_eq!(lines[1].start, (EYES_SHUT[0][0], EYES_SHUT[0][1]));
        assert_eq!(lines[2].segments, vec![Segment::Line((EYES_SHUT[1][2], EYES_SHUT[1][3]))]);
        // (The spark's top point, 2.6 above its middle at rest.)
        let top = |ms: f32| AI_SPARK_AT.1 - ai_waving(ms).1[0].start.1;
        assert!((top(300.0) - 0.3 * 2.6).abs() < 1e-3, "{}", top(300.0));
        assert!((500..700).any(|ms| top(ms as f32) > 2.6 * 1.05), "springs past its size");
        assert!((top(0.0) - 2.6).abs() < 1e-4);
    }

    #[test]
    fn every_icon_reads_whole() {
        for icon in [Icon::Cpu, Icon::Gpu, Icon::Memory, Icon::Disk, Icon::Network, Icon::Game, Icon::Pin, Icon::Tag, Icon::Close, Icon::Lock, Icon::Share, Icon::Settings, Icon::Check, Icon::Pointer, Icon::Panel, Icon::Restore, Icon::Update, Icon::Quit, Icon::Layers, Icon::Clock, Icon::ImagePlus, Icon::SkipBack, Icon::Play, Icon::Pause, Icon::SkipForward, Icon::Music, Icon::Volume, Icon::Muted, Icon::File, Icon::Link, Icon::Inbox, Icon::Clipboard, Icon::Ellipsis, Icon::List, Icon::Image, Icon::Add, Icon::Folder, Icon::ArrowDown, Icon::Search, Icon::Palette, Icon::Smartphone, Icon::Monitor, Icon::Megaphone, Icon::PencilRuler, Icon::PinOff, Icon::Trash, Icon::EyeOff, Icon::Bold, Icon::Italic, Icon::Strikethrough, Icon::Code, Icon::Scissors, Icon::Copy, Icon::ClipboardPaste, Icon::SquareCheck, Icon::Table, Icon::Minus, Icon::SquareCode, Icon::Heading1, Icon::Heading2, Icon::Heading3, Icon::Pilcrow, Icon::Text, Icon::ArrowUpToLine, Icon::ArrowDownToLine, Icon::ArrowLeftToLine, Icon::ArrowRightToLine, Icon::SquarePen, Icon::Unlink, Icon::Send, Icon::Paperclip, Icon::CircleDot, Icon::MessageCircle, Icon::CircleCheck, Icon::CircleX, Icon::AppWindow, Icon::Wrench, Icon::StickyNote, Icon::ExternalLink, Icon::ChevronLeft, Icon::Calendar, Icon::ChevronUp, Icon::ChevronRight, Icon::MoveDiagonal, Icon::MoveDiagonal2, Icon::MonitorSmartphone, Icon::Laptop, Icon::Speaker, Icon::Headphones, Icon::Headset, Icon::AudioOut, Icon::Earbuds, Icon::PortableSpeaker, Icon::Keyboard, Icon::Mouse, Icon::Watch, Icon::Bluetooth, Icon::Ai, Icon::Calculator, Icon::Globe] {
            let figures = icon.figures();
            assert!(!figures.is_empty(), "{icon:?}");
            // Every point on the grid, what is filled too.
            for figure in figures.iter().chain(&icon.spots()) {
                let (x, y) = figure.start;
                assert!((0.0..=24.0).contains(&x) && (0.0..=24.0).contains(&y), "{icon:?} {figure:?}");
            }
        }
        assert_eq!(Icon::Cpu.figures().len(), 14);
        assert_eq!(Icon::Gpu.figures().len(), 5);
        // The gear's twelve arcs, one command letter for all of them.
        assert_eq!(Icon::Settings.figures()[0].segments.len(), 12);
    }
}
