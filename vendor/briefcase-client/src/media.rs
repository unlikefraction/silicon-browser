//! Naming the media type of a file about to be uploaded.
//!
//! Briefcase stores whatever it is given and records the media type the client
//! declares, and that type decides which renderer a viewer opens and whether
//! the text is indexed for search. Guessing from the extension is the client's
//! job, so every client guesses the same way.

/// Media type used when nothing better is known.
pub const DEFAULT_CONTENT_TYPE: &str = "application/octet-stream";

const KNOWN_TYPES: &[(&str, &str)] = &[
    ("3gp", "video/3gpp"),
    ("7z", "application/x-7z-compressed"),
    ("aac", "audio/aac"),
    ("aiff", "audio/aiff"),
    ("amr", "audio/amr"),
    ("avi", "video/x-msvideo"),
    ("avif", "image/avif"),
    ("bmp", "image/bmp"),
    ("bz2", "application/x-bzip2"),
    ("css", "text/css"),
    ("csv", "text/csv"),
    ("doc", "application/msword"),
    (
        "docx",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    ),
    ("epub", "application/epub+zip"),
    ("flac", "audio/flac"),
    ("gif", "image/gif"),
    ("gz", "application/gzip"),
    ("gzip", "application/gzip"),
    ("heic", "image/heic"),
    ("htm", "text/html"),
    ("html", "text/html"),
    ("ico", "image/vnd.microsoft.icon"),
    ("iso", "application/x-iso9660-image"),
    ("java", "text/x-java-source"),
    ("jpeg", "image/jpeg"),
    ("jpg", "image/jpeg"),
    ("js", "text/javascript"),
    ("json", "application/json"),
    ("key", "application/vnd.apple.keynote"),
    ("log", "text/plain"),
    ("m4a", "audio/mp4"),
    ("m4v", "video/x-m4v"),
    ("md", "text/markdown"),
    ("mkv", "video/x-matroska"),
    ("mov", "video/quicktime"),
    ("mp3", "audio/mpeg"),
    ("mp4", "video/mp4"),
    ("mpeg", "video/mpeg"),
    ("mpg", "video/mpeg"),
    ("numbers", "application/vnd.apple.numbers"),
    ("odp", "application/vnd.oasis.opendocument.presentation"),
    ("ods", "application/vnd.oasis.opendocument.spreadsheet"),
    ("odt", "application/vnd.oasis.opendocument.text"),
    ("ogg", "audio/ogg"),
    ("ogv", "video/ogg"),
    ("opus", "audio/opus"),
    ("pages", "application/vnd.apple.pages"),
    ("pdf", "application/pdf"),
    ("png", "image/png"),
    ("ppt", "application/vnd.ms-powerpoint"),
    (
        "pptx",
        "application/vnd.openxmlformats-officedocument.presentationml.presentation",
    ),
    ("py", "text/x-python"),
    ("rar", "application/vnd.rar"),
    ("rs", "text/x-rust"),
    ("rtf", "application/rtf"),
    ("sql", "application/sql"),
    ("svg", "image/svg+xml"),
    ("tar", "application/x-tar"),
    ("tex", "application/x-tex"),
    ("tgz", "application/gzip"),
    ("tiff", "image/tiff"),
    ("toml", "application/toml"),
    ("ts", "text/typescript"),
    ("tsv", "text/tab-separated-values"),
    ("txt", "text/plain"),
    ("wav", "audio/wav"),
    ("webm", "video/webm"),
    ("webp", "image/webp"),
    ("wma", "audio/x-ms-wma"),
    ("xls", "application/vnd.ms-excel"),
    (
        "xlsx",
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    ),
    ("xml", "application/xml"),
    ("xz", "application/x-xz"),
    ("yaml", "application/yaml"),
    ("yml", "application/yaml"),
    ("zip", "application/zip"),
];

/// Guesses a file's media type from its name.
///
/// Unknown extensions get `application/octet-stream`, which Briefcase stores
/// and serves exactly the same way — it simply has no renderer and no text to
/// index.
#[must_use]
pub fn guess_content_type(file_name: &str) -> &'static str {
    let extension = file_name
        .rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase());
    let Some(extension) = extension else {
        return DEFAULT_CONTENT_TYPE;
    };
    KNOWN_TYPES
        .iter()
        .find(|(known, _)| *known == extension)
        .map_or(DEFAULT_CONTENT_TYPE, |(_, media_type)| *media_type)
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_CONTENT_TYPE, guess_content_type};

    #[test]
    fn a_known_extension_names_its_type_whatever_its_case() {
        assert_eq!(guess_content_type("notes.md"), "text/markdown");
        assert_eq!(guess_content_type("REPORT.PDF"), "application/pdf");
        assert_eq!(guess_content_type("archive.tar.gz"), "application/gzip");
    }

    #[test]
    fn anything_else_is_stored_as_opaque_bytes() {
        assert_eq!(guess_content_type("mystery.xyz"), DEFAULT_CONTENT_TYPE);
        assert_eq!(guess_content_type("Makefile"), DEFAULT_CONTENT_TYPE);
    }

    #[test]
    fn every_documented_render_extension_has_an_explicit_media_type() {
        for extension in [
            "png", "jpg", "jpeg", "webp", "gif", "svg", "avif", "heic", "tiff", "bmp", "ico",
            "mp4", "mov", "webm", "mkv", "avi", "mpeg", "mpg", "m4v", "3gp", "ogv", "pdf", "doc",
            "docx", "odt", "rtf", "txt", "md", "pages", "tex", "xls", "xlsx", "csv", "tsv", "ods",
            "numbers", "ppt", "pptx", "odp", "key", "mp3", "wav", "m4a", "aac", "flac", "ogg",
            "opus", "wma", "aiff", "amr", "zip", "rar", "7z", "tar", "gz", "gzip", "bz2", "xz",
            "tgz", "iso", "json", "xml", "yaml", "yml", "html", "css", "js", "ts", "py", "java",
            "sql", "log",
        ] {
            assert_ne!(
                guess_content_type(&format!("file.{extension}")),
                DEFAULT_CONTENT_TYPE,
                ".{extension} must not be uploaded as opaque bytes"
            );
        }
    }

    #[test]
    fn ambiguous_documented_extensions_use_the_product_rendering_meaning() {
        assert_eq!(guess_content_type("clip.3gp"), "video/3gpp");
        assert_eq!(guess_content_type("clip.avi"), "video/x-msvideo");
        assert_eq!(guess_content_type("source.ts"), "text/typescript");
        assert_eq!(guess_content_type("query.sql"), "application/sql");
        assert_eq!(guess_content_type("events.log"), "text/plain");
    }
}
