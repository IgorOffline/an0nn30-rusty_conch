//! File browser state for the left sidebar.

use std::path::PathBuf;

/// Which pane is active in the file browser when focused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FileBrowserPane {
    #[default]
    Local,
    Remote,
    Local2,
}

/// Which optional columns are visible in file browser tables.
#[derive(Debug, Clone)]
pub struct ColumnVisibility {
    pub ext: bool,
    pub size: bool,
    pub modified: bool,
}

impl Default for ColumnVisibility {
    fn default() -> Self {
        Self { ext: true, size: true, modified: true }
    }
}

/// State for the file browser panel.
#[derive(Debug, Clone)]
pub struct FileBrowserState {
    /// Whether the file browser has keyboard focus.
    pub focused: bool,
    /// Which pane (local/remote) is active for keyboard navigation.
    pub active_pane: FileBrowserPane,
    pub local_path: PathBuf,
    pub local_entries: Vec<FileListEntry>,
    pub local_path_edit: String,
    pub local_back_stack: Vec<PathBuf>,
    pub local_forward_stack: Vec<PathBuf>,
    pub local_selected: Option<usize>,
    pub remote_path: Option<PathBuf>,
    pub remote_entries: Vec<FileListEntry>,
    pub remote_path_edit: String,
    pub remote_back_stack: Vec<PathBuf>,
    pub remote_forward_stack: Vec<PathBuf>,
    pub remote_selected: Option<usize>,
    /// Second local pane (shown when no remote session is active).
    pub local2_path: PathBuf,
    pub local2_entries: Vec<FileListEntry>,
    pub local2_path_edit: String,
    pub local2_back_stack: Vec<PathBuf>,
    pub local2_forward_stack: Vec<PathBuf>,
    pub local2_selected: Option<usize>,
    /// Which optional columns are visible in file tables.
    pub columns: ColumnVisibility,
}

/// A single file or directory entry.
#[derive(Debug, Clone)]
pub struct FileListEntry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub size: u64,
    pub modified: Option<u64>,
}

impl From<conch_session::FileEntry> for FileListEntry {
    fn from(e: conch_session::FileEntry) -> Self {
        Self {
            name: e.name,
            path: e.path,
            is_dir: e.is_dir,
            size: e.size,
            modified: e.modified,
        }
    }
}

impl Default for FileBrowserState {
    fn default() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        let local_path_edit = home.to_string_lossy().into_owned();
        let local2_path_edit = local_path_edit.clone();
        Self {
            focused: false,
            active_pane: FileBrowserPane::default(),
            local_path: home.clone(),
            local_entries: Vec::new(),
            local_path_edit,
            local_back_stack: Vec::new(),
            local_forward_stack: Vec::new(),
            local_selected: None,
            remote_path: None,
            remote_entries: Vec::new(),
            remote_path_edit: String::new(),
            remote_back_stack: Vec::new(),
            remote_forward_stack: Vec::new(),
            remote_selected: None,
            local2_path: home,
            local2_entries: Vec::new(),
            local2_path_edit,
            local2_back_stack: Vec::new(),
            local2_forward_stack: Vec::new(),
            local2_selected: None,
            columns: ColumnVisibility::default(),
        }
    }
}

/// Format a byte count as a human-readable size string.
pub fn display_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;

    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

/// Recursively copy a directory and its contents.
/// Skips symlinks to avoid infinite recursion from cyclic links.
// TODO: prompt for confirmation before overwriting existing files.
pub fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let dest_path = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_recursive(&entry.path(), &dest_path)?;
        } else if ty.is_file() {
            std::fs::copy(entry.path(), &dest_path)?;
        }
        // symlinks are silently skipped
    }
    Ok(())
}

/// Return a human-readable description for a file extension (lowercase, no dot).
/// Returns `None` for unrecognised extensions.
pub fn extension_label(ext: &str) -> Option<&'static str> {
    match ext {
        // Documents
        "pdf"   => Some("PDF Document"),
        "doc"   => Some("MS Word Document"),
        "docx"  => Some("MS Word Document"),
        "xls"   => Some("MS Excel Spreadsheet"),
        "xlsx"  => Some("MS Excel Spreadsheet"),
        "ppt"   => Some("MS PowerPoint"),
        "pptx"  => Some("MS PowerPoint"),
        "odt"   => Some("OpenDocument Text"),
        "ods"   => Some("OpenDocument Spreadsheet"),
        "odp"   => Some("OpenDocument Presentation"),
        "rtf"   => Some("Rich Text Format"),
        "tex"   => Some("LaTeX Document"),
        "txt"   => Some("Text File"),
        "csv"   => Some("CSV File"),
        "tsv"   => Some("TSV File"),
        "md"    => Some("Markdown"),
        "rst"   => Some("reStructuredText"),
        "epub"  => Some("EPUB Book"),

        // Images
        "png"   => Some("PNG Image"),
        "jpg" | "jpeg" => Some("JPEG Image"),
        "gif"   => Some("GIF Image"),
        "bmp"   => Some("Bitmap Image"),
        "svg"   => Some("SVG Image"),
        "ico"   => Some("Icon File"),
        "icns"  => Some("macOS Icon"),
        "webp"  => Some("WebP Image"),
        "tiff" | "tif" => Some("TIFF Image"),
        "psd"   => Some("Photoshop File"),
        "raw"   => Some("RAW Image"),
        "heic" | "heif" => Some("HEIF Image"),
        "avif"  => Some("AVIF Image"),

        // Audio
        "mp3"   => Some("MP3 Audio"),
        "wav"   => Some("WAV Audio"),
        "flac"  => Some("FLAC Audio"),
        "ogg"   => Some("OGG Audio"),
        "aac"   => Some("AAC Audio"),
        "m4a"   => Some("M4A Audio"),
        "wma"   => Some("WMA Audio"),
        "aiff" | "aif" => Some("AIFF Audio"),
        "mid" | "midi" => Some("MIDI Audio"),
        "opus"  => Some("Opus Audio"),

        // Video
        "mp4"   => Some("MP4 Video"),
        "mkv"   => Some("MKV Video"),
        "avi"   => Some("AVI Video"),
        "mov"   => Some("QuickTime Video"),
        "wmv"   => Some("WMV Video"),
        "flv"   => Some("Flash Video"),
        "webm"  => Some("WebM Video"),
        "m4v"   => Some("M4V Video"),
        "mpg" | "mpeg" => Some("MPEG Video"),
        "3gp"   => Some("3GP Video"),

        // Archives
        "zip"   => Some("ZIP Archive"),
        "tar"   => Some("Tar Archive"),
        "gz"    => Some("Gzip Archive"),
        "bz2"   => Some("Bzip2 Archive"),
        "xz"    => Some("XZ Archive"),
        "zst"   => Some("Zstd Archive"),
        "7z"    => Some("7-Zip Archive"),
        "rar"   => Some("RAR Archive"),
        "dmg"   => Some("macOS Disk Image"),
        "iso"   => Some("ISO Disk Image"),
        "deb"   => Some("Debian Package"),
        "rpm"   => Some("RPM Package"),
        "apk"   => Some("Android Package"),
        "cab"   => Some("Windows Cabinet"),
        "msi"   => Some("Windows Installer"),

        // Shell / scripts
        "sh"    => Some("Shell Script"),
        "bash"  => Some("Bash Script"),
        "zsh"   => Some("Zsh Script"),
        "fish"  => Some("Fish Script"),
        "bat"   => Some("Win Batch Script"),
        "cmd"   => Some("Win Command Script"),
        "ps1"   => Some("PowerShell Script"),
        "psm1"  => Some("PowerShell Module"),

        // Programming – compiled
        "rs"    => Some("Rust Source"),
        "c"     => Some("C Source"),
        "h"     => Some("C Header"),
        "cpp" | "cc" | "cxx" => Some("C++ Source"),
        "hpp" | "hh" | "hxx" => Some("C++ Header"),
        "cs"    => Some("C# Source"),
        "java"  => Some("Java Source"),
        "class" => Some("Java Class"),
        "jar"   => Some("Java Archive"),
        "kt"    => Some("Kotlin Source"),
        "go"    => Some("Go Source"),
        "swift" => Some("Swift Source"),
        "m"     => Some("Objective-C Source"),
        "mm"    => Some("Objective-C++ Source"),
        "zig"   => Some("Zig Source"),
        "asm" | "s" => Some("Assembly Source"),
        "d"     => Some("D Source"),
        "v"     => Some("Verilog Source"),
        "vhd" | "vhdl" => Some("VHDL Source"),

        // Programming – interpreted / scripting
        "py"    => Some("Python Script"),
        "pyi"   => Some("Python Stub"),
        "pyc"   => Some("Python Bytecode"),
        "pyw"   => Some("Python Script (Win)"),
        "rb"    => Some("Ruby Script"),
        "pl"    => Some("Perl Script"),
        "pm"    => Some("Perl Module"),
        "lua"   => Some("Lua Script"),
        "r"     => Some("R Script"),
        "jl"    => Some("Julia Script"),
        "ex" | "exs" => Some("Elixir Source"),
        "erl"   => Some("Erlang Source"),
        "hs"    => Some("Haskell Source"),
        "ml"    => Some("OCaml Source"),
        "mli"   => Some("OCaml Interface"),
        "clj"   => Some("Clojure Source"),
        "scala" => Some("Scala Source"),
        "groovy"=> Some("Groovy Source"),
        "dart"  => Some("Dart Source"),
        "nim"   => Some("Nim Source"),
        "tcl"   => Some("Tcl Script"),

        // Web / markup
        "html" | "htm" => Some("HTML Document"),
        "css"   => Some("CSS Stylesheet"),
        "scss"  => Some("SCSS Stylesheet"),
        "sass"  => Some("Sass Stylesheet"),
        "less"  => Some("LESS Stylesheet"),
        "js"    => Some("JavaScript"),
        "mjs"   => Some("ES Module"),
        "cjs"   => Some("CommonJS Module"),
        "ts"    => Some("TypeScript"),
        "tsx"   => Some("TypeScript JSX"),
        "jsx"   => Some("React JSX"),
        "vue"   => Some("Vue Component"),
        "svelte"=> Some("Svelte Component"),
        "wasm"  => Some("WebAssembly"),
        "php"   => Some("PHP Script"),
        "asp" | "aspx" => Some("ASP.NET Page"),
        "jsp"   => Some("Java Server Page"),
        "erb"   => Some("ERB Template"),

        // Data / config
        "json"  => Some("JSON File"),
        "jsonl" => Some("JSON Lines"),
        "json5" => Some("JSON5 File"),
        "yaml" | "yml" => Some("YAML File"),
        "toml"  => Some("TOML File"),
        "xml"   => Some("XML File"),
        "xsl" | "xslt" => Some("XSL Transform"),
        "ini"   => Some("INI Config"),
        "cfg"   => Some("Config File"),
        "conf"  => Some("Config File"),
        "env"   => Some("Environment File"),
        "properties" => Some("Properties File"),
        "plist" => Some("Property List"),
        "sql"   => Some("SQL Script"),
        "sqlite" | "db" => Some("SQLite Database"),
        "graphql" | "gql" => Some("GraphQL Schema"),
        "proto" => Some("Protobuf Schema"),
        "avro"  => Some("Avro Schema"),
        "parquet" => Some("Parquet File"),
        "ndjson"=> Some("Newline JSON"),

        // DevOps / infra
        "dockerfile" => Some("Dockerfile"),
        "tf"    => Some("Terraform Config"),
        "hcl"   => Some("HCL Config"),
        "vagrantfile" => Some("Vagrantfile"),

        // Build / project
        "makefile" | "mk" => Some("Makefile"),
        "cmake" => Some("CMake Script"),
        "gradle"=> Some("Gradle Build"),
        "sln"   => Some("VS Solution"),
        "csproj"=> Some("C# Project"),
        "vcxproj" => Some("VC++ Project"),
        "xcodeproj" | "xcworkspace" => Some("Xcode Project"),

        // Fonts
        "ttf"   => Some("TrueType Font"),
        "otf"   => Some("OpenType Font"),
        "woff"  => Some("WOFF Font"),
        "woff2" => Some("WOFF2 Font"),
        "eot"   => Some("EOT Font"),

        // Certificates / keys
        "pem"   => Some("PEM Certificate"),
        "crt" | "cer" => Some("Certificate"),
        "key"   => Some("Private Key"),
        "csr"   => Some("Certificate Request"),
        "p12" | "pfx" => Some("PKCS#12 Cert"),
        "pub"   => Some("Public Key"),

        // Misc
        "log"   => Some("Log File"),
        "bak"   => Some("Backup File"),
        "tmp" | "temp" => Some("Temporary File"),
        "lock"  => Some("Lock File"),
        "pid"   => Some("PID File"),
        "swp"   => Some("Vim Swap File"),
        "o"     => Some("Object File"),
        "a"     => Some("Static Library"),
        "so"    => Some("Shared Library"),
        "dylib" => Some("macOS Dyn Library"),
        "dll"   => Some("Windows DLL"),
        "lib"   => Some("Windows Library"),
        "exe"   => Some("Windows Executable"),
        "app"   => Some("macOS Application"),
        "bin"   => Some("Binary File"),
        "dat"   => Some("Data File"),
        "patch" => Some("Patch File"),
        "diff"  => Some("Diff File"),

        _ => None,
    }
}

/// Format an optional UNIX timestamp as a short date string.
pub fn format_modified(timestamp: Option<u64>) -> String {
    match timestamp {
        Some(ts) => {
            let dt = chrono::DateTime::from_timestamp(ts as i64, 0);
            match dt {
                Some(dt) => dt.format("%Y-%m-%d %H:%M").to_string(),
                None => "—".to_string(),
            }
        }
        None => "—".to_string(),
    }
}
