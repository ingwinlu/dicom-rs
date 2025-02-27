#![allow(clippy::derive_partial_eq_without_eq)]
//! DICOM data dumping library
//!
//! This is a helper library
//! for dumping the contents of DICOM objects and elements
//! in a human readable way.
//!
//! # Examples
//!
//! A quick and easy way to dump the contents of a DICOM object
//! is via [`dump_file`]
//! (or [`dump_file_to`] to print to an arbitrary writer).
//!
//! ```no_run
//! use dicom_object::open_file;
//! use dicom_dump::dump_file;
//!
//! let obj = open_file("path/to/file.dcm")?;
//! dump_file(&obj)?;
//! # Result::<(), Box<dyn std::error::Error>>::Ok(())
//! ```
//!
//! See the [`DumpOptions`] builder for additional dumping options.
//!
//! ```no_run
//! use dicom_object::open_file;
//! use dicom_dump::{DumpOptions, dump_file};
//!
//! let obj = open_file("path/to/file2.dcm")?;
//! let mut options = DumpOptions::new();
//! // dump to stdout (width = 100)
//! options.width(100).dump_file(&obj)?;
//! # Result::<(), Box<dyn std::error::Error>>::Ok(())
//! ```
use colored::*;
use dicom_core::dictionary::{DataDictionary, DictionaryEntry};
use dicom_core::header::Header;
use dicom_core::value::{PrimitiveValue, Value as DicomValue};
use dicom_core::VR;
use dicom_encoding::transfer_syntax::TransferSyntaxIndex;
use dicom_object::mem::{InMemDicomObject, InMemElement};
use dicom_object::{FileDicomObject, FileMetaTable, StandardDataDictionary};
use dicom_transfer_syntax_registry::TransferSyntaxRegistry;
use std::borrow::Cow;
use std::fmt::{self, Display, Formatter};
use std::io::{stdout, Result as IoResult, Write};
use std::str::FromStr;

/// An enum of all supported output formats for dumping DICOM data.
#[derive(Debug, Copy, Clone, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum DumpFormat {
    /// The main DICOM dump format adopted by the project.
    ///
    /// It is primarily designed to be human readable,
    /// although its output can be used to recover the original object
    /// in its uncut form (no limit width).
    /// It makes a distinction between single value and multi-value elements,
    /// and displays the tag, alias, and VR of each element.
    ///
    /// Note that this format is not stabilized,
    /// and may change with subsequent versions of the crate.
    Main,
}

/// The [main output format](DumpFormat::Main) is used by default.
impl Default for DumpFormat {
    fn default() -> Self {
        DumpFormat::Main
    }
}

/// Options and flags to configure how to dump a DICOM file or object.
///
/// This is a builder which exposes the various options available
/// for printing the contents of the DICOM file in a readable way.
///
/// Once set up,
/// the [`dump_file`] or [`dump_file_to`] methods can be used
/// to finalize the DICOM data dumping process on an open file.
/// Both file meta table and main data set are dumped.
/// Alternatively,
/// [`dump_object`] or [`dump_object_to`] methods
/// work on bare DICOM objects without a file meta table.
///
/// [`dump_file`]: DumpOptions::dump_file
/// [`dump_file_to`]: DumpOptions::dump_file_to
/// [`dump_object`]: DumpOptions::dump_object
/// [`dump_object_to`]: DumpOptions::dump_object_to
///
/// # Example
///
/// ```no_run
/// use dicom_object::open_file;
/// use dicom_dump::{ColorMode, DumpOptions};
///
/// let my_dicom_file = open_file("/path_to_file")?;
/// let mut options = DumpOptions::new();
/// options
///     // maximum 120 characters per line
///     .width(120)
///     // no limit for text values
///     .no_text_limit(true)
///     // never print colored output
///     .color_mode(ColorMode::Never)
///     // dump to stdout
///     .dump_file(&my_dicom_file)?;
/// # Result::<(), Box<dyn std::error::Error>>::Ok(())
/// ```
#[derive(Debug, Default, Clone, PartialEq)]
#[non_exhaustive]
pub struct DumpOptions {
    /// the output format
    pub format: DumpFormat,
    /// whether to produce colored output
    pub color: ColorMode,
    /// the console width to assume when trimming long values
    pub width: Option<u32>,
    /// never trim out long text values
    pub no_text_limit: bool,
    /// never trim out any values (implies `no_text_limit`)
    pub no_limit: bool,
}

impl DumpOptions {
    pub fn new() -> Self {
        Default::default()
    }

    /// Set the output format.
    ///
    /// See the [`DumpFormat`] documentation for the list of supported formats.
    pub fn format(&mut self, format: DumpFormat) -> &mut Self {
        self.format = format;
        self
    }

    /// Set the maximum output width in number of characters.
    pub fn width(&mut self, width: u32) -> &mut Self {
        self.width = Some(width);
        self
    }

    /// Set the maximum output width to automatic,
    /// based on terminal size.
    ///
    /// This is the default behavior.
    /// If a terminal width could not be determined,
    /// the default width of 120 characters is used.
    pub fn width_auto(&mut self) -> &mut Self {
        self.width = None;
        self
    }

    /// Set whether to remove the maximum width restriction for text values.
    pub fn no_text_limit(&mut self, no_text_limit: bool) -> &mut Self {
        self.no_text_limit = no_text_limit;
        self
    }

    /// Set whether to remove the maximum width restriction
    /// for all DICOM values.
    pub fn no_limit(&mut self, no_limit: bool) -> &mut Self {
        self.no_limit = no_limit;
        self
    }

    /// Set the output color mode.
    pub fn color_mode(&mut self, color: ColorMode) -> &mut Self {
        self.color = color;
        self
    }

    /// Dump the contents of an open DICOM file to standard output.
    pub fn dump_file<D>(&self, obj: &FileDicomObject<InMemDicomObject<D>>) -> IoResult<()>
    where
        D: DataDictionary,
    {
        self.dump_file_to(stdout(), obj)
    }

    /// Dump the contents of an open DICOM file to the given writer.
    pub fn dump_file_to<D>(
        &self,
        mut to: impl Write,
        obj: &FileDicomObject<InMemDicomObject<D>>,
    ) -> IoResult<()>
    where
        D: DataDictionary,
    {
        match self.color {
            ColorMode::Never => colored::control::set_override(false),
            ColorMode::Always => colored::control::set_override(true),
            ColorMode::Auto => colored::control::unset_override(),
        }

        let meta = obj.meta();

        let width = determine_width(self.width);

        meta_dump(&mut to, meta, if self.no_limit { u32::MAX } else { width })?;

        writeln!(to, "{:-<58}", "")?;

        dump(&mut to, obj, width, 0, self.no_text_limit, self.no_limit)?;

        Ok(())
    }

    /// Dump the contents of a DICOM object to standard output.
    #[inline]
    pub fn dump_object<D>(&self, obj: &InMemDicomObject<D>) -> IoResult<()>
    where
        D: DataDictionary,
    {
        self.dump_object_impl(stdout(), obj, true)
    }

    /// Dump the contents of a DICOM object to the given writer.
    #[inline]
    pub fn dump_object_to<D>(&self, to: impl Write, obj: &InMemDicomObject<D>) -> IoResult<()>
    where
        D: DataDictionary,
    {
        self.dump_object_impl(to, obj, false)
    }

    fn dump_object_impl<D>(
        &self,
        mut to: impl Write,
        obj: &InMemDicomObject<D>,
        to_stdout: bool,
    ) -> IoResult<()>
    where
        D: DataDictionary,
    {
        match (self.color, to_stdout) {
            (ColorMode::Never, _) => colored::control::set_override(false),
            (ColorMode::Always, _) => colored::control::set_override(true),
            (ColorMode::Auto, false) => colored::control::set_override(false),
            (ColorMode::Auto, true) => colored::control::unset_override(),
        }

        let width = if let Some((width, _)) = term_size::dimensions() {
            width as u32
        } else {
            120
        };

        dump(&mut to, obj, width, 0, self.no_text_limit, self.no_limit)?;

        Ok(())
    }
}

/// Enumeration of output coloring modes.
#[derive(Debug, Copy, Clone, Eq, Hash, PartialEq)]
pub enum ColorMode {
    /// Produce colored output if supported by the destination
    /// (namely, if the destination is a terminal).
    /// When calling [`dump_file_to`](DumpOptions::dump_file_to)
    /// or [`dump_object_to`](DumpOptions::dump_object_to),
    /// the output will not be colored.
    ///
    /// This is the default behavior.
    Auto,
    /// Never produce colored output.
    Never,
    /// Always produce colored output.
    Always,
}

impl Default for ColorMode {
    fn default() -> Self {
        ColorMode::Auto
    }
}

impl std::fmt::Display for ColorMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ColorMode::Never => f.write_str("never"),
            ColorMode::Auto => f.write_str("auto"),
            ColorMode::Always => f.write_str("always"),
        }
    }
}

impl FromStr for ColorMode {
    type Err = ColorModeError;
    fn from_str(color: &str) -> Result<Self, Self::Err> {
        match color {
            "never" => Ok(ColorMode::Never),
            "auto" => Ok(ColorMode::Auto),
            "always" => Ok(ColorMode::Always),
            _ => Err(ColorModeError),
        }
    }
}

/// The error raised when providing an invalid color mode.
#[derive(Debug, Default, Copy, Clone, Eq, Hash, PartialEq)]
pub struct ColorModeError;

impl Display for ColorModeError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str("invalid color mode")
    }
}

impl std::error::Error for ColorModeError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DumpValue<T>
where
    T: ToString,
{
    TagNum(T),
    Alias(T),
    Num(T),
    Str(T),
    DateTime(T),
    Invalid(T),
    Nothing,
}

impl<T> fmt::Display for DumpValue<T>
where
    T: ToString,
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let value = match self {
            DumpValue::TagNum(v) => v.to_string().dimmed(),
            DumpValue::Alias(v) => v.to_string().bold(),
            DumpValue::Num(v) => v.to_string().cyan(),
            DumpValue::Str(v) => v.to_string().yellow(),
            DumpValue::DateTime(v) => v.to_string().green(),
            DumpValue::Invalid(v) => v.to_string().red(),
            DumpValue::Nothing => "(no value)".italic(),
        };
        if let Some(width) = f.width() {
            write!(f, "{:width$}", value, width = width)
        } else {
            write!(f, "{}", value)
        }
    }
}

/// Dump the contents of a DICOM file to stdout.
///
/// Both file meta table and main data set are dumped.
pub fn dump_file<D>(obj: &FileDicomObject<InMemDicomObject<D>>) -> IoResult<()>
where
    D: DataDictionary,
{
    DumpOptions::new().dump_file(obj)
}

/// Dump the contents of a DICOM file to the given writer.
///
/// Both file meta table and main data set are dumped.
pub fn dump_file_to<D>(to: impl Write, obj: &FileDicomObject<InMemDicomObject<D>>) -> IoResult<()>
where
    D: DataDictionary,
{
    DumpOptions::new().dump_file_to(to, obj)
}

/// Dump the contents of a DICOM object to stdout.
pub fn dump_object<D>(obj: &InMemDicomObject<D>) -> IoResult<()>
where
    D: DataDictionary,
{
    DumpOptions::new().dump_object(obj)
}

/// Dump the contents of a DICOM object to the given writer.
pub fn dump_object_to<D>(to: impl Write, obj: &InMemDicomObject<D>) -> IoResult<()>
where
    D: DataDictionary,
{
    DumpOptions::new().dump_object_to(to, obj)
}

#[inline]
fn whitespace_or_null(c: char) -> bool {
    c.is_whitespace() || c == '\0'
}

fn meta_dump<W>(to: &mut W, meta: &FileMetaTable, width: u32) -> IoResult<()>
where
    W: ?Sized + Write,
{
    writeln!(
        to,
        "{}: {}",
        "Media Storage SOP Class UID".bold(),
        meta.media_storage_sop_class_uid
            .trim_end_matches(whitespace_or_null),
    )?;
    writeln!(
        to,
        "{}: {}",
        "Media Storage SOP Instance UID".bold(),
        meta.media_storage_sop_instance_uid
            .trim_end_matches(whitespace_or_null),
    )?;
    if let Some(ts) = TransferSyntaxRegistry.get(&meta.transfer_syntax) {
        writeln!(
            to,
            "{}: {} ({})",
            "Transfer Syntax".bold(),
            ts.uid(),
            ts.name()
        )?;
    } else {
        writeln!(
            to,
            "{}: {} («UNKNOWN»)",
            "Transfer Syntax".bold(),
            meta.transfer_syntax.trim_end_matches(whitespace_or_null)
        )?;
    }
    writeln!(
        to,
        "{}: {}",
        "Implementation Class UID".bold(),
        meta.implementation_class_uid
            .trim_end_matches(whitespace_or_null),
    )?;

    if let Some(v) = meta.implementation_version_name.as_ref() {
        writeln!(
            to,
            "{}: {}",
            "Implementation version name".bold(),
            v.trim_end()
        )?;
    }

    if let Some(v) = meta.source_application_entity_title.as_ref() {
        writeln!(
            to,
            "{}: {}",
            "Source Application Entity Title".bold(),
            v.trim_end()
        )?;
    }

    if let Some(v) = meta.sending_application_entity_title.as_ref() {
        writeln!(
            to,
            "{}: {}",
            "Sending Application Entity Title".bold(),
            v.trim_end()
        )?;
    }

    if let Some(v) = meta.receiving_application_entity_title.as_ref() {
        writeln!(
            to,
            "{}: {}",
            "Receiving Application Entity Title".bold(),
            v.trim_end()
        )?;
    }

    if let Some(v) = meta.private_information_creator_uid.as_ref() {
        writeln!(
            to,
            "{}: {}",
            "Private Information Creator UID".bold(),
            v.trim_end_matches(whitespace_or_null)
        )?;
    }

    if let Some(v) = meta.private_information.as_ref() {
        writeln!(
            to,
            "{}: {}",
            "Private Information".bold(),
            format_value_list(v.iter().map(|n| format!("{:02X}", n)), Some(width), false)
        )?;
    }

    writeln!(to)?;
    Ok(())
}

fn dump<W, D>(
    to: &mut W,
    obj: &InMemDicomObject<D>,
    width: u32,
    depth: u32,
    no_text_limit: bool,
    no_limit: bool,
) -> IoResult<()>
where
    W: ?Sized + Write,
    D: DataDictionary,
{
    for elem in obj {
        dump_element(&mut *to, elem, width, depth, no_text_limit, no_limit)?;
    }

    Ok(())
}

pub fn dump_element<W, D>(
    to: &mut W,
    elem: &InMemElement<D>,
    width: u32,
    depth: u32,
    no_text_limit: bool,
    no_limit: bool,
) -> IoResult<()>
where
    W: ?Sized + Write,
    D: DataDictionary,
{
    let indent = vec![b' '; (depth * 2) as usize];
    let tag_alias = StandardDataDictionary
        .by_tag(elem.tag())
        .map(DictionaryEntry::alias)
        .unwrap_or("«Unknown Attribute»");
    to.write_all(&indent)?;
    let vm = match elem.vr() {
        VR::OB | VR::OW | VR::UN => 1,
        _ => elem.value().multiplicity(),
    };

    match elem.value() {
        DicomValue::Sequence { items, .. } => {
            writeln!(
                to,
                "{} {:28} {} ({} Item{})",
                DumpValue::TagNum(elem.tag()),
                DumpValue::Alias(tag_alias),
                elem.vr(),
                vm,
                if vm == 1 { "" } else { "s" },
            )?;
            for item in items {
                dump_item(&mut *to, item, width, depth + 2, no_text_limit, no_limit)?;
            }
            to.write_all(&indent)?;
            writeln!(
                to,
                "{} {}",
                DumpValue::TagNum("(FFFE,E0DD)"),
                DumpValue::Alias("SequenceDelimitationItem"),
            )?;
        }
        DicomValue::PixelSequence {
            fragments,
            offset_table,
        } => {
            // write pixel sequence start line
            let vr = elem.vr();
            let num_items = 1 + fragments.len();
            writeln!(
                to,
                "{} {:28} {} (PixelSequence, {} Item{})",
                DumpValue::TagNum(elem.tag()),
                "PixelData".bold(),
                vr,
                num_items,
                if num_items == 1 { "" } else { "s" },
            )?;

            // write offset table
            let byte_len = offset_table.len();
            let summary = offset_table_summary(
                offset_table,
                Some(width)
                    .filter(|_| !no_limit)
                    .map(|w| w.saturating_sub(38 + depth * 2)),
            );
            writeln!(
                to,
                "  {} offset table ({:>3} bytes, 1 Item): {:48}",
                DumpValue::TagNum("(FFFE,E000)"),
                byte_len,
                summary,
            )?;

            // write compressed fragments
            for fragment in fragments {
                let byte_len = fragment.len();
                let summary = item_value_summary(
                    fragment,
                    Some(width)
                        .filter(|_| !no_limit)
                        .map(|w| w.saturating_sub(38 + depth * 2)),
                );
                writeln!(
                    to,
                    "  {} pi ({:>3} bytes, 1 Item): {:48}",
                    DumpValue::TagNum("(FFFE,E000)"),
                    byte_len,
                    summary
                )?;
            }
        }
        DicomValue::Primitive(value) => {
            let vr = elem.vr();
            let byte_len = elem.header().len.0;
            writeln!(
                to,
                "{} {:28} {} ({},{:>3} bytes): {}",
                DumpValue::TagNum(elem.tag()),
                DumpValue::Alias(tag_alias),
                vr,
                vm,
                byte_len,
                value_summary(
                    value,
                    vr,
                    width.saturating_sub(63 + depth * 2),
                    no_text_limit,
                    no_limit,
                ),
            )?;
        }
    }

    Ok(())
}

fn dump_item<W, D>(
    to: &mut W,
    item: &InMemDicomObject<D>,
    width: u32,
    depth: u32,
    no_text_limit: bool,
    no_limit: bool,
) -> IoResult<()>
where
    W: ?Sized + Write,
    D: DataDictionary,
{
    let indent: String = "  ".repeat(depth as usize);
    writeln!(
        to,
        "{}{} na {}",
        indent,
        DumpValue::TagNum("(FFFE,E000)"),
        DumpValue::Alias("Item"),
    )?;
    dump(to, item, width, depth + 1, no_text_limit, no_limit)?;
    writeln!(
        to,
        "{}{} {}",
        indent,
        DumpValue::TagNum("(FFFE,E00D)"),
        DumpValue::Alias("ItemDelimitationItem"),
    )?;
    Ok(())
}

fn value_summary(
    value: &PrimitiveValue,
    vr: VR,
    max_characters: u32,
    no_text_limit: bool,
    no_limit: bool,
) -> DumpValue<String> {
    use PrimitiveValue::*;

    let max_characters = match (no_limit, no_text_limit, vr) {
        (true, _, _) => None,
        (
            false,
            true,
            VR::CS
            | VR::AE
            | VR::DA
            | VR::DS
            | VR::DT
            | VR::IS
            | VR::LO
            | VR::LT
            | VR::PN
            | VR::TM
            | VR::UC
            | VR::UI
            | VR::UR,
        ) => None,
        (false, _, _) => Some(max_characters),
    };
    match (value, vr) {
        (F32(values), _) => DumpValue::Num(format_value_list(values, max_characters, false)),
        (F64(values), _) => DumpValue::Num(format_value_list(values, max_characters, false)),
        (I32(values), _) => DumpValue::Num(format_value_list(values, max_characters, false)),
        (I64(values), _) => DumpValue::Num(format_value_list(values, max_characters, false)),
        (U32(values), _) => DumpValue::Num(format_value_list(values, max_characters, false)),
        (U64(values), _) => DumpValue::Num(format_value_list(values, max_characters, false)),
        (I16(values), _) => DumpValue::Num(format_value_list(values, max_characters, false)),
        (U16(values), VR::OW) => DumpValue::Num(format_value_list(
            values.into_iter().map(|n| format!("{:02X}", n)),
            max_characters,
            false,
        )),
        (U16(values), _) => DumpValue::Num(format_value_list(values, max_characters, false)),
        (U8(values), VR::OB) | (U8(values), VR::UN) => DumpValue::Num(format_value_list(
            values.into_iter().map(|n| format!("{:02X}", n)),
            max_characters,
            false,
        )),
        (U8(values), _) => DumpValue::Num(format_value_list(values, max_characters, false)),
        (Tags(values), _) => DumpValue::Str(format_value_list(values, max_characters, false)),
        (Strs(values), VR::DA) => {
            match value.to_multi_date() {
                Ok(values) => {
                    // print as reformatted date
                    DumpValue::DateTime(format_value_list(values, max_characters, false))
                }
                Err(_e) => {
                    // print as text
                    DumpValue::Invalid(format_value_list(values, max_characters, true))
                }
            }
        }
        (Strs(values), VR::TM) => {
            match value.to_multi_time() {
                Ok(values) => {
                    // print as reformatted date
                    DumpValue::DateTime(format_value_list(values, max_characters, false))
                }
                Err(_e) => {
                    // print as text
                    DumpValue::Invalid(format_value_list(values, max_characters, true))
                }
            }
        }
        (Strs(values), VR::DT) => {
            match value.to_multi_datetime(dicom_core::chrono::FixedOffset::east_opt(0).unwrap()) {
                Ok(values) => {
                    // print as reformatted date
                    DumpValue::DateTime(format_value_list(values, max_characters, false))
                }
                Err(_e) => {
                    // print as text
                    DumpValue::Invalid(format_value_list(values, max_characters, true))
                }
            }
        }
        (Strs(values), _) => DumpValue::Str(format_value_list(
            values
                .iter()
                .map(|s| s.trim_end_matches(whitespace_or_null)),
            max_characters,
            true,
        )),
        (Date(values), _) => DumpValue::DateTime(format_value_list(values, max_characters, true)),
        (Time(values), _) => DumpValue::DateTime(format_value_list(values, max_characters, true)),
        (DateTime(values), _) => {
            DumpValue::DateTime(format_value_list(values, max_characters, true))
        }
        (Str(value), _) => {
            let txt = format!(
                "\"{}\"",
                value.to_string().trim_end_matches(whitespace_or_null)
            );
            if let Some(max) = max_characters {
                DumpValue::Str(cut_str(&txt, max).to_string())
            } else {
                DumpValue::Str(txt)
            }
        }
        (Empty, _) => DumpValue::Nothing,
    }
}

fn item_value_summary(data: &[u8], max_characters: Option<u32>) -> DumpValue<String> {
    DumpValue::Num(format_value_list(
        data.iter().map(|n| format!("{:02X}", n)),
        max_characters,
        false,
    ))
}

fn offset_table_summary(data: &[u32], max_characters: Option<u32>) -> String {
    if data.is_empty() {
        format!("{}", "(empty)".italic())
    } else {
        format_value_list(
            data.iter().map(|n| format!("{:02X}", n)),
            max_characters,
            false,
        )
    }
}

fn format_value_list<I>(values: I, max_characters: Option<u32>, quoted: bool) -> String
where
    I: IntoIterator,
    I::IntoIter: ExactSizeIterator,
    I::Item: std::fmt::Display,
{
    let values = values.into_iter();
    let len = values.len();
    let mut acc_size = 0;
    let mut pieces = String::new();
    if len > 1 {
        pieces.push('[');
    }
    for piece in values {
        let mut piece = piece.to_string();
        piece = piece.replace(|c: char| c.is_control(), "�");
        if acc_size > 0 {
            pieces.push_str(", ");
        }

        if quoted {
            piece = piece.replace('\"', "\\\"");
            pieces.push('"');
        }

        acc_size += piece.len();
        pieces.push_str(&piece);
        if quoted {
            pieces.push('"');
        }
        // stop earlier if applicable
        if max_characters
            .filter(|max| (*max as usize) < acc_size)
            .is_some()
        {
            break;
        }
    }
    if len > 1 {
        pieces.push(']');
    }
    if let Some(max_characters) = max_characters {
        cut_str(&pieces, max_characters).into_owned()
    } else {
        pieces
    }
}

fn cut_str(s: &str, max_characters: u32) -> Cow<str> {
    let max = (max_characters.saturating_sub(3)) as usize;
    let len = s.chars().count();

    if len > max {
        s.chars()
            .take(max)
            .chain("...".chars())
            .collect::<String>()
            .into()
    } else {
        s.into()
    }
}

fn determine_width(user_width: Option<u32>) -> u32 {
    user_width
        .or_else(|| term_size::dimensions().map(|(w, _)| w as u32))
        .unwrap_or(120)
}

#[cfg(test)]
mod tests {

    use dicom_core::{DataElement, PrimitiveValue, VR};
    use dicom_dictionary_std::tags;
    use dicom_object::{FileMetaTableBuilder, InMemDicomObject};

    use super::whitespace_or_null;
    use crate::{ColorMode, DumpOptions};

    #[test]
    fn trims_all_whitespace() {
        assert_eq!("   ".trim_end_matches(whitespace_or_null), "");
        assert_eq!("\0".trim_end_matches(whitespace_or_null), "");
        assert_eq!("1.4.5.6\0".trim_end_matches(whitespace_or_null), "1.4.5.6");
        assert_eq!("AETITLE ".trim_end_matches(whitespace_or_null), "AETITLE");
    }

    #[test]
    fn dump_file_to_covers_properties() {
        // create object
        let obj = InMemDicomObject::from_element_iter(vec![DataElement::new(
            tags::SOP_INSTANCE_UID,
            VR::UI,
            PrimitiveValue::from("1.2.888.123"),
        )]);

        let file = obj
            .with_meta(
                FileMetaTableBuilder::new()
                    // Implicit VR Little Endian
                    .transfer_syntax("1.2.840.10008.1.2")
                    // Computed Radiography Image Storage
                    .media_storage_sop_class_uid("1.2.840.10008.5.1.4.1.1.1"),
            )
            .unwrap();

        let mut out = Vec::new();
        DumpOptions::new()
            .color_mode(ColorMode::Never)
            .dump_file_to(&mut out, &file)
            .unwrap();

        let lines: Vec<_> = std::str::from_utf8(&out)
            .expect("output is not valid UTF-8")
            .split('\n')
            .collect();
        assert_eq!(
            lines[0],
            "Media Storage SOP Class UID: 1.2.840.10008.5.1.4.1.1.1"
        );
        assert_eq!(lines[1], "Media Storage SOP Instance UID: 1.2.888.123");
        assert_eq!(
            lines[2],
            "Transfer Syntax: 1.2.840.10008.1.2 (Implicit VR Little Endian)"
        );
        assert!(lines[3].starts_with("Implementation Class UID: "));
        assert!(lines[4].starts_with("Implementation version name: "));
        assert_eq!(lines[5], "");
        assert_eq!(
            lines[6],
            "----------------------------------------------------------"
        );

        let parts: Vec<&str> = lines[7].split(" ").filter(|p| !p.is_empty()).collect();
        assert_eq!(&parts[..3], &["(0008,0018)", "SOPInstanceUID", "UI"]);
    }

    #[test]
    fn dump_object_to_covers_properties() {
        // create object
        let obj = InMemDicomObject::from_element_iter(vec![DataElement::new(
            tags::SOP_INSTANCE_UID,
            VR::UI,
            PrimitiveValue::from("1.2.888.123"),
        )]);

        let mut out = Vec::new();
        DumpOptions::new()
            .color_mode(ColorMode::Never)
            .dump_object_to(&mut out, &obj)
            .unwrap();

        let lines: Vec<_> = std::str::from_utf8(&out)
            .expect("output is not valid UTF-8")
            .split('\n')
            .collect();
        let parts: Vec<&str> = lines[0].split(" ").filter(|p| !p.is_empty()).collect();

        assert_eq!(&parts[..3], &["(0008,0018)", "SOPInstanceUID", "UI"]);
    }
}
