//! Streaming EPUB cleaning pipeline.
//!
//! A single EPUB is written sequentially because the ZIP central directory is
//! emitted at the end. Rayon is used by the batch API for independent EPUBs.

use anyhow::{bail, Context, Result};
use lol_html::{element, HtmlRewriter, Settings};
use quick_xml::events::Event;
use rayon::prelude::*;
use serde::Serialize;
use std::cell::RefCell;
use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::rc::Rc;
use tempfile::NamedTempFile;
use url::Url;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

const MIME_TYPE: &[u8] = b"application/epub+zip";
const COPY_BUFFER_SIZE: usize = 64 * 1024;

#[derive(Debug, Clone)]
pub struct CleanOptions {
    pub remove_external_links: bool,
    pub remove_ad_containers: bool,
    pub max_entry_bytes: u64,
}

impl Default for CleanOptions {
    fn default() -> Self {
        Self {
            remove_external_links: true,
            remove_ad_containers: true,
            max_entry_bytes: 256 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct CleanReport {
    pub input: String,
    pub output: String,
    pub entries: usize,
    pub transformed_entries: usize,
    pub bytes_in: u64,
    pub bytes_out: u64,
    pub external_links_removed: u64,
    pub ad_containers_removed: u64,
}

#[derive(Debug, Default, Clone)]
struct TransformStats {
    external_links_removed: u64,
    ad_containers_removed: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum CleanError {
    #[error("invalid EPUB: {0}")]
    InvalidEpub(String),
    #[error("entry exceeds configured safety limit: {0}")]
    EntryTooLarge(String),
}

/// Clean one EPUB atomically. The destination is replaced only after the
/// complete archive has been written and validated.
pub fn clean_epub(input: &Path, output: &Path, options: &CleanOptions) -> Result<CleanReport> {
    if input == output || same_file(input, output) {
        bail!("input and output must be different files");
    }
    let input_file = File::open(input).with_context(|| format!("open {}", input.display()))?;
    let mut archive = ZipArchive::new(input_file).context("read input ZIP archive")?;
    validate_input_container(&mut archive, options.max_entry_bytes)?;

    let output_parent = output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(output_parent)
        .with_context(|| format!("create output directory {}", output_parent.display()))?;
    let mut temporary = NamedTempFile::new_in(output_parent)
        .with_context(|| format!("create temporary output in {}", output_parent.display()))?;

    let mut report = CleanReport {
        input: input.display().to_string(),
        output: output.display().to_string(),
        ..Default::default()
    };
    {
        let mut writer = ZipWriter::new(BufWriter::new(temporary.as_file_mut()));
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index).context("read ZIP entry")?;
            let name = entry.name().to_string();
            validate_entry_name(&name)?;
            report.entries += 1;
            report.bytes_in = report.bytes_in.saturating_add(entry.size());
            if entry.size() > options.max_entry_bytes {
                return Err(CleanError::EntryTooLarge(name).into());
            }

            if index == 0 {
                let file_options =
                    SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
                writer.start_file("mimetype", file_options)?;
                writer.write_all(MIME_TYPE)?;
                report.bytes_out = report.bytes_out.saturating_add(MIME_TYPE.len() as u64);
                continue;
            }

            let is_markup = is_markup_entry(&name);
            if is_markup && !entry.is_dir() {
                let mut transformed = NamedTempFile::new_in(output_parent)?;
                let stats = transform_markup(&mut entry, transformed.as_file_mut(), options)?;
                transformed.as_file_mut().flush()?;
                transformed.as_file_mut().seek(SeekFrom::Start(0))?;
                validate_xml_if_needed(&name, transformed.as_file_mut())?;
                transformed.as_file_mut().seek(SeekFrom::Start(0))?;
                writer.start_file(&name, file_options_for(&name))?;
                let copied = io::copy(transformed.as_file_mut(), &mut writer)?;
                report.bytes_out = report.bytes_out.saturating_add(copied);
                report.transformed_entries += 1;
                report.external_links_removed += stats.external_links_removed;
                report.ad_containers_removed += stats.ad_containers_removed;
            } else if is_xml_entry(&name) && !entry.is_dir() {
                let mut xml_copy = NamedTempFile::new_in(output_parent)?;
                io::copy(&mut entry, xml_copy.as_file_mut())?;
                xml_copy.as_file_mut().flush()?;
                xml_copy.as_file_mut().seek(SeekFrom::Start(0))?;
                validate_xml_reader(xml_copy.as_file_mut(), &name)?;
                xml_copy.as_file_mut().seek(SeekFrom::Start(0))?;
                writer.start_file(&name, file_options_for(&name))?;
                let copied = io::copy(xml_copy.as_file_mut(), &mut writer)?;
                report.bytes_out = report.bytes_out.saturating_add(copied);
            } else {
                let file_options = file_options_for(&name);
                if entry.is_dir() {
                    writer.add_directory(&name, file_options)?;
                } else {
                    writer.start_file(&name, file_options)?;
                    let copied = io::copy(&mut entry, &mut writer)?;
                    report.bytes_out = report.bytes_out.saturating_add(copied);
                }
            }
        }
        writer.finish()?;
    }
    temporary.as_file_mut().flush()?;
    temporary.as_file_mut().sync_all().ok();
    validate_output_container(temporary.path(), options.max_entry_bytes)?;
    temporary.persist(output).map_err(|error| error.error)?;
    Ok(report)
}

/// Process independent EPUB files concurrently.
pub fn clean_batch(
    input_dir: &Path,
    output_dir: &Path,
    options: &CleanOptions,
) -> Result<Vec<CleanReport>> {
    let mut inputs = fs::read_dir(input_dir)
        .with_context(|| format!("read input directory {}", input_dir.display()))?
        .filter_map(|item| item.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("epub")))
        .collect::<Vec<_>>();
    inputs.sort();
    fs::create_dir_all(output_dir)?;

    inputs
        .par_iter()
        .map(|input| {
            let output = output_dir.join(input.file_name().expect("input has filename"));
            clean_epub(input, &output, options)
                .with_context(|| format!("clean {}", input.display()))
        })
        .collect::<Result<Vec<_>>>()
}

fn transform_markup<R: Read, W: Write>(
    input: &mut R,
    output: W,
    options: &CleanOptions,
) -> Result<TransformStats> {
    let stats = Rc::new(RefCell::new(TransformStats::default()));
    let link_stats = Rc::clone(&stats);
    let ad_stats = Rc::clone(&stats);
    let remove_external_links = options.remove_external_links;
    let remove_ad_containers = options.remove_ad_containers;
    let mut output = BufWriter::new(output);
    let mut rewriter = HtmlRewriter::new(
        Settings {
            element_content_handlers: vec![
                element!("a[href]", move |element| {
                    if remove_external_links {
                        if let Some(href) = element.get_attribute("href") {
                            if is_external_href(&href) {
                                element.remove_and_keep_content();
                                link_stats.borrow_mut().external_links_removed += 1;
                            }
                        }
                    }
                    Ok(())
                }),
                element!(
                    "div,section,article,aside,header,footer,p,table,ul,ol",
                    move |element| {
                        if remove_ad_containers {
                            let id = element.get_attribute("id").unwrap_or_default();
                            let class = element.get_attribute("class").unwrap_or_default();
                            if is_ad_marker(&id) || is_ad_marker(&class) {
                                // Unwrap matched containers instead of deleting
                                // descendants. This is the image-safe fallback:
                                // an illustration inside a noisy wrapper survives.
                                element.remove_and_keep_content();
                                ad_stats.borrow_mut().ad_containers_removed += 1;
                            }
                        }
                        Ok(())
                    }
                ),
            ],
            ..Settings::default()
        },
        move |chunk: &[u8]| -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
            output
                .write_all(chunk)
                .map_err(|error| Box::new(error) as Box<dyn std::error::Error + Send + Sync>)
        },
    );
    let mut buffer = [0_u8; COPY_BUFFER_SIZE];
    loop {
        let read = input.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        rewriter.write(&buffer[..read])?;
    }
    rewriter.end()?;
    output.flush()?;
    match Rc::try_unwrap(stats) {
        Ok(cell) => Ok(cell.into_inner()),
        Err(shared) => Ok(shared.borrow().clone()),
    }
}

fn validate_input_container(archive: &mut ZipArchive<File>, max_entry_bytes: u64) -> Result<()> {
    if archive.len() == 0 {
        return Err(CleanError::InvalidEpub("archive is empty".into()).into());
    }
    {
        let mut first = archive.by_index(0)?;
        if first.name() != "mimetype" {
            return Err(
                CleanError::InvalidEpub("mimetype must be the first ZIP entry".into()).into(),
            );
        }
        if first.compression() != CompressionMethod::Stored
            || first.size() != MIME_TYPE.len() as u64
        {
            return Err(
                CleanError::InvalidEpub("mimetype must be stored and exact-length".into()).into(),
            );
        }
        let mut mime = Vec::new();
        (&mut first).take(max_entry_bytes).read_to_end(&mut mime)?;
        if mime != MIME_TYPE {
            return Err(
                CleanError::InvalidEpub("mimetype content is not application/epub+zip".into())
                    .into(),
            );
        }
    }
    let mut names = HashSet::new();
    for index in 0..archive.len() {
        let entry = archive.by_index(index)?;
        validate_entry_name(entry.name())?;
        if !names.insert(entry.name().to_string()) {
            return Err(
                CleanError::InvalidEpub(format!("duplicate ZIP entry: {}", entry.name())).into(),
            );
        }
        if entry.size() > max_entry_bytes {
            return Err(CleanError::EntryTooLarge(entry.name().to_string()).into());
        }
    }
    let mut container = archive
        .by_name("META-INF/container.xml")
        .context("EPUB is missing META-INF/container.xml")?;
    validate_xml_reader(&mut container, "META-INF/container.xml")?;
    Ok(())
}

fn validate_output_container(path: &Path, max_entry_bytes: u64) -> Result<()> {
    let mut archive = ZipArchive::new(File::open(path)?)?;
    validate_input_container(&mut archive, max_entry_bytes)
}

fn validate_xml_if_needed(name: &str, file: &mut File) -> Result<()> {
    if name.ends_with(".xml")
        || name.ends_with(".opf")
        || name.ends_with(".ncx")
        || name.ends_with(".xhtml")
    {
        file.seek(SeekFrom::Start(0))?;
        validate_xml_reader(file, name)?;
    }
    Ok(())
}

fn validate_xml_reader<R: Read>(reader: &mut R, name: &str) -> Result<()> {
    let mut xml = quick_xml::Reader::from_reader(BufReader::new(reader));
    let mut buffer = Vec::new();
    loop {
        match xml.read_event_into(&mut buffer) {
            Ok(Event::Eof) => break,
            Ok(_) => buffer.clear(),
            Err(error) => {
                return Err(
                    CleanError::InvalidEpub(format!("malformed XML in {name}: {error}")).into(),
                );
            }
        }
    }
    Ok(())
}

fn file_options_for(name: &str) -> SimpleFileOptions {
    if name == "mimetype" {
        SimpleFileOptions::default().compression_method(CompressionMethod::Stored)
    } else {
        SimpleFileOptions::default().compression_method(CompressionMethod::Deflated)
    }
}

fn is_markup_entry(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".xhtml") || lower.ends_with(".html") || lower.ends_with(".htm")
}

fn is_xml_entry(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".xml")
        || lower.ends_with(".opf")
        || lower.ends_with(".ncx")
        || lower.ends_with(".xhtml")
}

fn validate_entry_name(name: &str) -> Result<()> {
    let path = Path::new(name);
    if name.is_empty() || path.is_absolute() || name.split('/').any(|part| part == "..") {
        return Err(CleanError::InvalidEpub(format!("unsafe ZIP entry name: {name:?}")).into());
    }
    Ok(())
}

fn same_file(left: &Path, right: &Path) -> bool {
    match (fs::canonicalize(left), fs::canonicalize(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

fn is_external_href(href: &str) -> bool {
    let value = href.trim();
    if value.starts_with("//") {
        return true;
    }
    Url::parse(value)
        .map(|url| matches!(url.scheme(), "http" | "https"))
        .unwrap_or(false)
}

fn is_ad_marker(value: &str) -> bool {
    value
        .split(|character: char| {
            character.is_ascii_whitespace() || matches!(character, '-' | '_' | ':' | '.')
        })
        .map(|token| token.to_ascii_lowercase())
        .any(|token| {
            matches!(
                token.as_str(),
                "ad"
                    | "ads"
                    | "advert"
                    | "advertisement"
                    | "sponsor"
                    | "sponsored"
                    | "banner"
                    | "promotion"
                    | "promoted"
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_internal_and_fragment_links() {
        assert!(!is_external_href("#footnote-1"));
        assert!(!is_external_href("chapter2.xhtml#p3"));
        assert!(!is_external_href("mailto:author@example.com"));
    }

    #[test]
    fn recognizes_external_http_links_only() {
        assert!(is_external_href("https://example.com/ad"));
        assert!(is_external_href("//cdn.example.com/a.js"));
        assert!(!is_external_href("images/cover.jpg"));
    }

    #[test]
    fn ad_detection_avoids_substrings() {
        assert!(is_ad_marker("ad-banner sponsored"));
        assert!(!is_ad_marker("address-card"));
        assert!(!is_ad_marker("chapter"));
    }

    #[test]
    fn streaming_rewrite_preserves_text_and_images() {
        let input = concat!(
            r#"<html><body><p class="ad-banner"><img src="cover.jpg">Offer</p>"#,
            r#"<a href="https://example.com">Chapter</a>"#,
            r#"<a href="#note-1">Footnote</a></body></html>"#,
        )
        .as_bytes();
        let mut output = Vec::new();
        let report =
            transform_markup(input.as_slice(), &mut output, &CleanOptions::default()).unwrap();
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("cover.jpg"));
        assert!(output.contains("Chapter"));
        assert!(output.contains("#note-1"));
        assert!(!output.contains("https://example.com"));
        assert_eq!(report.external_links_removed, 1);
        assert_eq!(report.ad_containers_removed, 1);
    }
}
