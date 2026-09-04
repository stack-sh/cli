//! Local-only provider icon archive import.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{self, Write as _};
use std::fs::{self, OpenOptions};
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};

use roxmltree::{Document, Node, NodeType};
use sha2::{Digest, Sha256};
use stack_theme::{
    ProviderIcon, ProviderIconAsset, ProviderNodeKind, ProviderPack, ProviderPackDistributionMode,
    ProviderPackSource, ProviderPackTransformation,
};
use zip::ZipArchive;

use crate::provider_catalog::{ProviderCatalog, provider_catalog, provider_catalogs};

const PROVIDER_PACK_SCHEMA: &str =
    "https://raw.githubusercontent.com/stack-sh/theme/main/schemas/provider-pack.schema.json";
const MAX_ARCHIVE_BYTES: u64 = 32 * 1024 * 1024;
const MAX_ICON_BYTES: u64 = 1024 * 1024;
const SVG_NAMESPACE: &str = "http://www.w3.org/2000/svg";
const XLINK_NAMESPACE: &str = "http://www.w3.org/1999/xlink";

const ALLOWED_ELEMENTS: &[&str] = &[
    "circle",
    "clipPath",
    "defs",
    "ellipse",
    "g",
    "line",
    "linearGradient",
    "mask",
    "path",
    "polygon",
    "polyline",
    "radialGradient",
    "rect",
    "stop",
    "svg",
];

const ALLOWED_ATTRIBUTES: &[&str] = &[
    "aria-hidden",
    "clip-path",
    "clip-rule",
    "cx",
    "cy",
    "d",
    "fill",
    "fill-opacity",
    "fill-rule",
    "fx",
    "fy",
    "gradientTransform",
    "gradientUnits",
    "height",
    "href",
    "id",
    "isolation",
    "mask",
    "maskUnits",
    "opacity",
    "offset",
    "points",
    "r",
    "role",
    "rx",
    "ry",
    "stop-color",
    "stop-opacity",
    "stroke",
    "stroke-linecap",
    "stroke-linejoin",
    "stroke-miterlimit",
    "stroke-width",
    "transform",
    "viewBox",
    "width",
    "x",
    "x1",
    "x2",
    "y",
    "y1",
    "y2",
];

/// Summary of one completed local provider-pack import.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ImportSummary {
    pub(crate) provider_name: String,
    pub(crate) icon_count: usize,
    pub(crate) manifest_path: PathBuf,
    pub(crate) notice_path: PathBuf,
}

/// Stable local import error without archive-library implementation details.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ImportError {
    message: String,
}

impl ImportError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for ImportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ImportError {}

struct ProcessedIcon {
    svg: String,
    view_box: [i32; 4],
    transformations: Vec<ProviderPackTransformation>,
}

struct ParsedViewBox {
    integers: [i32; 4],
    coordinate_scale: i32,
}

/// Imports one audited official archive into a new local pack directory.
pub(crate) fn import_provider_pack(
    provider: &str,
    archive_path: &Path,
    additional_archive_paths: &BTreeMap<String, PathBuf>,
    output_path: &Path,
) -> Result<ImportSummary, ImportError> {
    let profile = provider_catalog(provider).map_err(ImportError::new)?;
    import_profile(profile, archive_path, additional_archive_paths, output_path)
}

fn import_profile(
    profile: ProviderCatalog,
    archive_path: &Path,
    additional_archive_paths: &BTreeMap<String, PathBuf>,
    output_path: &Path,
) -> Result<ImportSummary, ImportError> {
    if output_path.exists() {
        return Err(ImportError::new(format!(
            "output '{}' already exists",
            output_path.display()
        )));
    }
    let expected_additional = profile
        .additional_sources
        .iter()
        .map(|source| source.id.as_str())
        .collect::<BTreeSet<_>>();
    if let Some(unexpected) = additional_archive_paths
        .keys()
        .find(|id| !expected_additional.contains(id.as_str()))
    {
        return Err(ImportError::new(format!(
            "provider '{}' does not declare an additional source '{unexpected}'",
            profile.provider.id
        )));
    }
    if let Some(missing) = expected_additional
        .iter()
        .find(|id| !additional_archive_paths.contains_key(**id))
    {
        return Err(ImportError::new(format!(
            "missing local archive for additional source '{missing}'"
        )));
    }

    let mut archives = BTreeMap::new();
    archives.insert(
        "primary".to_owned(),
        open_archive(archive_path, &profile.source)?,
    );
    for additional in &profile.additional_sources {
        let Some(path) = additional_archive_paths.get(&additional.id) else {
            return Err(ImportError::new("missing additional provider archive"));
        };
        archives.insert(
            additional.id.clone(),
            open_archive(path, &additional.source)?,
        );
    }
    let mut manifest_icons = Vec::with_capacity(profile.icons.len());
    let mut processed_assets = Vec::with_capacity(profile.icons.len());

    for icon in &profile.icons {
        let source_id = icon.source_id.as_deref().unwrap_or("primary");
        let Some(archive) = archives.get_mut(source_id) else {
            return Err(ImportError::new(format!(
                "icon '{}' references unknown source '{source_id}'",
                icon.id
            )));
        };
        let original = read_icon_entry(archive, &icon.archive_path)?;
        let original_sha256 = sha256(&original);
        let slug = icon
            .id
            .split_once(':')
            .map(|(_, slug)| slug)
            .ok_or_else(|| {
                ImportError::new(format!("catalog icon '{}' is not namespaced", icon.id))
            })?;
        let processed = sanitize_svg(&original, &profile.provider.id, slug).map_err(|error| {
            ImportError::new(format!(
                "cannot process catalog icon '{}': {error}",
                icon.id
            ))
        })?;
        let processed_sha256 = sha256(processed.svg.as_bytes());
        let asset_path = format!("assets/{slug}.svg");
        manifest_icons.push(ProviderIcon {
            id: icon.id.clone(),
            subject: icon.subject.clone(),
            product_name: icon.product_name.clone(),
            brand_source_url: icon.brand_source_url.clone(),
            brand_guidelines_url: icon.brand_guidelines_url.clone(),
            recommended_node_kind: icon.recommended_node_kind,
            asset: ProviderIconAsset {
                source_id: icon.source_id.clone(),
                path: asset_path.clone(),
                original_path: icon.archive_path.clone(),
                view_box: processed.view_box,
                original_sha256,
                processed_sha256,
                transformations: processed.transformations,
            },
        });
        processed_assets.push((asset_path, processed.svg));
    }

    let manifest =
        ProviderPack {
            schema: PROVIDER_PACK_SCHEMA.to_owned(),
            schema_version: if profile.additional_sources.is_empty()
                && profile.icons.iter().all(|icon| {
                    icon.brand_source_url.is_none() && icon.brand_guidelines_url.is_none()
                }) {
                "1.0".to_owned()
            } else {
                "1.1".to_owned()
            },
            pack_version: profile.pack_version.clone(),
            provider: profile.provider.clone(),
            distribution_mode: ProviderPackDistributionMode::UserImported,
            source: profile.source.clone(),
            additional_sources: profile.additional_sources.clone(),
            rights: profile.rights.clone(),
            notice: profile.notice.clone(),
            icons: manifest_icons,
        };
    let mut manifest_bytes = match serde_json::to_vec_pretty(&manifest) {
        Ok(bytes) => bytes,
        Err(_) => return Err(ImportError::new("cannot serialize provider manifest")),
    };
    manifest_bytes.push(b'\n');
    let notice = render_notice(&manifest);

    let parent = match output_path.parent() {
        Some(path) if !path.as_os_str().is_empty() => path,
        _ => Path::new("."),
    };
    if !parent.is_dir() {
        return Err(ImportError::new(format!(
            "output parent '{}' is not a directory",
            parent.display()
        )));
    }
    let temporary_path = create_temporary_directory(parent)?;
    let write_result = write_pack_directory(
        &temporary_path,
        &manifest_bytes,
        notice.as_bytes(),
        &processed_assets,
    );
    if let Err(error) = write_result {
        let _ = fs::remove_dir_all(&temporary_path);
        return Err(error);
    }
    if let Err(error) = fs::rename(&temporary_path, output_path) {
        let _ = fs::remove_dir_all(&temporary_path);
        return Err(ImportError::new(format!(
            "cannot create output '{}': {}",
            output_path.display(),
            stable_io_error(error.kind())
        )));
    }

    Ok(ImportSummary {
        provider_name: profile.provider.name,
        icon_count: manifest.icons.len(),
        manifest_path: output_path.join("manifest.json"),
        notice_path: output_path.join("NOTICE.md"),
    })
}

fn open_archive(
    archive_path: &Path,
    source: &ProviderPackSource,
) -> Result<ZipArchive<Cursor<Vec<u8>>>, ImportError> {
    let metadata = match fs::metadata(archive_path) {
        Ok(metadata) => metadata,
        Err(error) => {
            return Err(ImportError::new(format!(
                "cannot read archive '{}': {}",
                archive_path.display(),
                stable_io_error(error.kind())
            )));
        }
    };
    if !metadata.is_file() {
        return Err(ImportError::new(format!(
            "archive '{}' is not a file",
            archive_path.display()
        )));
    }
    if metadata.len() > MAX_ARCHIVE_BYTES {
        return Err(ImportError::new(format!(
            "archive '{}' exceeds the 32 MiB limit",
            archive_path.display()
        )));
    }
    let archive_bytes = match fs::read(archive_path) {
        Ok(bytes) => bytes,
        Err(error) => {
            return Err(ImportError::new(format!(
                "cannot read archive '{}': {}",
                archive_path.display(),
                stable_io_error(error.kind())
            )));
        }
    };
    let archive_digest = sha256(&archive_bytes);
    if archive_digest != source.archive_sha256 {
        return Err(ImportError::new(format!(
            "archive hash does not match the audited {} release; expected {}, received {}",
            source.release, source.archive_sha256, archive_digest
        )));
    }
    ZipArchive::new(Cursor::new(archive_bytes))
        .map_err(|_| ImportError::new("archive is not a supported ZIP file"))
}

pub(crate) fn render_catalog_list(
    provider: Option<&str>,
    query: Option<&str>,
) -> Result<String, ImportError> {
    if provider.is_none() {
        let catalogs = provider_catalogs().map_err(ImportError::new)?;
        let mut output = String::from("PROVIDER\tICONS\tRELEASE\n");
        for catalog in catalogs {
            let _ = writeln!(
                output,
                "{}\t{}\t{}",
                catalog.provider.id,
                catalog.icons.len(),
                catalog.source.release
            );
        }
        return Ok(output);
    }

    let catalog = provider_catalog(provider.unwrap_or_default()).map_err(ImportError::new)?;
    let query = query.unwrap_or_default().to_lowercase();
    let mut output = String::from("ID\tPRODUCT\tCATEGORY\tKIND\n");
    for icon in catalog.icons.iter().filter(|icon| {
        query.is_empty()
            || icon.id.to_lowercase().contains(&query)
            || icon.product_name.to_lowercase().contains(&query)
            || icon.category.to_lowercase().contains(&query)
    }) {
        let _ = writeln!(
            output,
            "{}\t{}\t{}\t{}",
            icon.id,
            icon.product_name,
            icon.category,
            node_kind_name(icon.recommended_node_kind)
        );
    }
    Ok(output)
}

fn node_kind_name(kind: ProviderNodeKind) -> &'static str {
    match kind {
        ProviderNodeKind::Actor => "actor",
        ProviderNodeKind::Client => "client",
        ProviderNodeKind::Service => "service",
        ProviderNodeKind::Function => "function",
        ProviderNodeKind::Worker => "worker",
        ProviderNodeKind::Database => "database",
        ProviderNodeKind::Cache => "cache",
        ProviderNodeKind::Queue => "queue",
        ProviderNodeKind::Storage => "storage",
        ProviderNodeKind::External => "external",
    }
}

fn read_icon_entry<R: Read + std::io::Seek>(
    archive: &mut ZipArchive<R>,
    entry_path: &str,
) -> Result<Vec<u8>, ImportError> {
    let mut entry = match archive.by_name(entry_path) {
        Ok(entry) => entry,
        Err(_) => {
            return Err(ImportError::new(format!(
                "archive is missing required icon '{entry_path}'"
            )));
        }
    };
    if entry.is_dir() || entry.is_symlink() {
        return Err(ImportError::new(format!(
            "archive icon '{entry_path}' is not a regular file"
        )));
    }
    if entry.enclosed_name().as_deref() != Some(Path::new(entry_path)) {
        return Err(ImportError::new(format!(
            "archive icon '{entry_path}' has an unsafe path"
        )));
    }
    if entry.size() > MAX_ICON_BYTES {
        return Err(ImportError::new(format!(
            "archive icon '{entry_path}' exceeds the 1 MiB limit"
        )));
    }
    let mut bytes = Vec::with_capacity(entry.size() as usize);
    let read_result = entry
        .by_ref()
        .take(MAX_ICON_BYTES + 1)
        .read_to_end(&mut bytes);
    if read_result.is_err() {
        return Err(ImportError::new(format!(
            "cannot decode archive icon '{entry_path}'"
        )));
    }
    if bytes.len() as u64 > MAX_ICON_BYTES {
        return Err(ImportError::new(format!(
            "archive icon '{entry_path}' exceeds the 1 MiB limit"
        )));
    }
    Ok(bytes)
}

fn sanitize_svg(
    original: &[u8],
    provider_id: &str,
    icon_slug: &str,
) -> Result<ProcessedIcon, ImportError> {
    let source = match std::str::from_utf8(original) {
        Ok(source) => source,
        Err(_) => return Err(ImportError::new("provider icon is not valid UTF-8 SVG")),
    };
    let source = strip_xml_declaration(source)?;
    let uppercase = source.to_ascii_uppercase();
    if uppercase.contains("<!DOCTYPE") || uppercase.contains("<!ENTITY") || source.contains("<?") {
        return Err(ImportError::new(
            "provider icon contains a forbidden XML declaration or entity",
        ));
    }
    let document = match Document::parse(source) {
        Ok(document) => document,
        Err(_) => return Err(ImportError::new("provider icon is not well-formed SVG")),
    };
    let root = document.root_element();
    if root.tag_name().name() != "svg" || root.tag_name().namespace() != Some(SVG_NAMESPACE) {
        return Err(ImportError::new(
            "provider icon root must use the canonical SVG namespace",
        ));
    }
    let view_box_value = match root
        .attribute("viewBox")
        .or_else(|| root.attribute("xviewBox"))
    {
        Some(value) => value,
        None => return Err(ImportError::new("provider icon is missing viewBox")),
    };
    let view_box = parse_view_box(view_box_value)?;
    let styles = collect_styles(&document)?;
    let (identifier_map, unused_identifiers) =
        identifier_map(&document, &styles, provider_id, icon_slug)?;
    let mut removed_metadata = false;
    for node in document.descendants() {
        if matches!(node.node_type(), NodeType::Comment | NodeType::PI)
            || (node.is_element() && matches!(node.tag_name().name(), "title" | "desc"))
        {
            removed_metadata = true;
            break;
        }
    }
    let mut state = SanitizeState {
        styles: &styles,
        identifiers: &identifier_map,
        removed_metadata,
        inlined_styles: !styles.is_empty(),
        removed_unused_identifiers: unused_identifiers,
        view_box: view_box.integers,
        coordinate_scale: view_box.coordinate_scale,
    };
    let svg = match serialize_element(root, true, None, &mut state)? {
        Some(svg) => svg,
        None => {
            return Err(ImportError::new(
                "provider icon has no renderable SVG content",
            ));
        }
    };
    let mut transformations = Vec::new();
    if state.removed_metadata {
        transformations.push(ProviderPackTransformation::RemoveMetadata);
    }
    if state.inlined_styles {
        transformations.push(ProviderPackTransformation::InlineStyles);
    }
    if state.removed_unused_identifiers {
        transformations.push(ProviderPackTransformation::RemoveUnusedIdentifiers);
    }
    if !identifier_map.is_empty() {
        transformations.push(ProviderPackTransformation::NamespaceIdentifiers);
    }
    if view_box.coordinate_scale != 1 {
        transformations.push(ProviderPackTransformation::ScaleViewBoxToIntegers);
    }
    transformations.push(ProviderPackTransformation::NormalizeXml);

    Ok(ProcessedIcon {
        svg: format!("{svg}\n"),
        view_box: view_box.integers,
        transformations,
    })
}

fn strip_xml_declaration(source: &str) -> Result<&str, ImportError> {
    let trimmed = source.trim_start_matches('\u{feff}').trim_start();
    let Some(after_prefix) = trimmed.strip_prefix("<?xml") else {
        return Ok(trimmed);
    };
    if !after_prefix.chars().next().is_some_and(char::is_whitespace) {
        return Ok(trimmed);
    }
    let end = match trimmed.find("?>") {
        Some(end) => end,
        None => {
            return Err(ImportError::new(
                "provider icon has an incomplete XML declaration",
            ));
        }
    };
    Ok(trimmed[end + 2..].trim_start())
}

fn parse_view_box(value: &str) -> Result<ParsedViewBox, ImportError> {
    let components = value
        .split(|character: char| character.is_ascii_whitespace() || character == ',')
        .filter(|component| !component.is_empty())
        .collect::<Vec<_>>();
    if components.len() != 4 {
        return Err(ImportError::new(
            "provider icon viewBox must contain four finite decimal numbers",
        ));
    }
    let decimal_places = components
        .iter()
        .map(|component| decimal_places(component))
        .collect::<Result<Vec<_>, _>>()?;
    let max_decimal_places = decimal_places.iter().copied().max().unwrap_or(0);
    debug_assert!(max_decimal_places <= 6);
    let coordinate_scale = 10_i32.pow(max_decimal_places);
    let mut integers = [0_i32; 4];
    for (index, component) in components.iter().enumerate() {
        integers[index] = scaled_decimal(component, max_decimal_places)?;
    }
    if integers[2] <= 0 || integers[3] <= 0 {
        return Err(ImportError::new(
            "provider icon viewBox must have positive dimensions",
        ));
    }
    Ok(ParsedViewBox {
        integers,
        coordinate_scale,
    })
}

fn decimal_places(value: &str) -> Result<u32, ImportError> {
    let unsigned = value
        .strip_prefix('-')
        .or_else(|| value.strip_prefix('+'))
        .unwrap_or(value);
    let (integer, fraction) = unsigned.split_once('.').unwrap_or((unsigned, ""));
    if integer.is_empty()
        || !integer.bytes().all(|byte| byte.is_ascii_digit())
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
        || fraction.len() > 6
    {
        return Err(ImportError::new(
            "provider icon viewBox must contain finite decimals with at most six places",
        ));
    }
    debug_assert!(fraction.len() <= 6);
    Ok(fraction.len() as u32)
}

fn scaled_decimal(value: &str, decimal_places: u32) -> Result<i32, ImportError> {
    let negative = value.starts_with('-');
    let unsigned = value
        .strip_prefix('-')
        .or_else(|| value.strip_prefix('+'))
        .unwrap_or(value);
    let (integer, fraction) = unsigned.split_once('.').unwrap_or((unsigned, ""));
    let integer = integer
        .parse::<i64>()
        .map_err(|_| ImportError::new("provider icon viewBox is outside the supported range"))?;
    let fraction_value = if fraction.is_empty() {
        0_i64
    } else {
        fraction
            .parse::<i64>()
            .map_err(|_| ImportError::new("provider icon viewBox is invalid"))?
    };
    debug_assert!(fraction.len() <= 6);
    let fraction_places = fraction.len() as u32;
    debug_assert!(decimal_places <= 6);
    debug_assert!(fraction_places <= decimal_places);
    let scale = 10_i64.pow(decimal_places);
    let fraction_scale = 10_i64.pow(decimal_places - fraction_places);
    let magnitude = integer
        .checked_mul(scale)
        .and_then(|value| value.checked_add(fraction_value * fraction_scale))
        .ok_or_else(|| ImportError::new("provider icon viewBox is outside the supported range"))?;
    let signed = if negative { -magnitude } else { magnitude };
    i32::try_from(signed)
        .map_err(|_| ImportError::new("provider icon viewBox is outside the supported range"))
}

fn collect_styles(document: &Document<'_>) -> Result<BTreeMap<String, String>, ImportError> {
    let mut styles = BTreeMap::new();
    for node in document.descendants() {
        if !node.is_element() || node.tag_name().name() != "style" {
            continue;
        }
        let stylesheet = match node.text() {
            Some(stylesheet) => stylesheet,
            None => return Err(ImportError::new("provider stylesheet is empty")),
        };
        for block in stylesheet.split('}') {
            let block = block.trim();
            if block.is_empty() {
                continue;
            }
            let (selector, declarations) = match block.split_once('{') {
                Some(parts) => parts,
                None => {
                    return Err(ImportError::new("provider stylesheet has invalid syntax"));
                }
            };
            let class_name = match selector.trim().strip_prefix('.') {
                Some(class_name) => class_name,
                None => {
                    return Err(ImportError::new(
                        "provider stylesheet selector is not a class",
                    ));
                }
            };
            if class_name.is_empty()
                || !class_name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
            {
                return Err(ImportError::new(
                    "provider stylesheet has an invalid class name",
                ));
            }
            let mut fill = None;
            for declaration in declarations.split(';') {
                let declaration = declaration.trim();
                if declaration.is_empty() {
                    continue;
                }
                let (property, value) = match declaration.split_once(':') {
                    Some(parts) => parts,
                    None => {
                        return Err(ImportError::new(
                            "provider stylesheet declaration is invalid",
                        ));
                    }
                };
                if property.trim() != "fill" || fill.is_some() {
                    return Err(ImportError::new(
                        "provider stylesheet may define one fill property per class",
                    ));
                }
                let value = value.trim();
                if local_url_reference(value).is_none() {
                    validate_safe_value(value)?;
                }
                fill = Some(value.to_owned());
            }
            let fill = match fill {
                Some(fill) => fill,
                None => {
                    return Err(ImportError::new(
                        "provider stylesheet class does not define fill",
                    ));
                }
            };
            if styles.insert(class_name.to_owned(), fill).is_some() {
                return Err(ImportError::new(
                    "provider stylesheet declares a duplicate class",
                ));
            }
        }
    }
    Ok(styles)
}

fn identifier_map(
    document: &Document<'_>,
    styles: &BTreeMap<String, String>,
    provider_id: &str,
    icon_slug: &str,
) -> Result<(BTreeMap<String, String>, bool), ImportError> {
    let mut declared = BTreeMap::new();
    let mut referenced = BTreeSet::new();
    for node in document.descendants() {
        if !node.is_element() {
            continue;
        }
        if let Some(identifier) = node.attribute("id") {
            *declared.entry(identifier.to_owned()).or_insert(0_usize) += 1;
        }
        for attribute in node.attributes() {
            if let Some(identifier) = local_url_reference(attribute.value()) {
                referenced.insert(identifier.to_owned());
            } else if attribute.name() == "href" {
                if let Some(identifier) = fragment_reference(attribute.value()) {
                    referenced.insert(identifier.to_owned());
                }
            } else if attribute.value().to_ascii_lowercase().contains("url(") {
                return Err(ImportError::new(
                    "provider icon contains a non-local URL reference",
                ));
            }
        }
    }
    for value in styles.values() {
        if let Some(identifier) = local_url_reference(value) {
            referenced.insert(identifier.to_owned());
        }
    }
    for identifier in &referenced {
        if declared.get(identifier) != Some(&1) {
            return Err(ImportError::new(
                "provider icon references an undeclared or duplicate identifier",
            ));
        }
    }
    let mut map = BTreeMap::new();
    for (index, identifier) in referenced.iter().enumerate() {
        map.insert(
            identifier.clone(),
            format!("stack-{provider_id}-{icon_slug}-gradient-{index}"),
        );
    }
    let mut has_unused_identifier = false;
    for identifier in declared.keys() {
        if !referenced.contains(identifier) {
            has_unused_identifier = true;
            break;
        }
    }
    Ok((map, has_unused_identifier))
}

fn local_url_reference(value: &str) -> Option<&str> {
    let value = value.strip_prefix("url(#")?;
    let value = value.strip_suffix(')')?;
    if value.is_empty() { None } else { Some(value) }
}

fn fragment_reference(value: &str) -> Option<&str> {
    value.strip_prefix('#').filter(|value| !value.is_empty())
}

struct SanitizeState<'a> {
    styles: &'a BTreeMap<String, String>,
    identifiers: &'a BTreeMap<String, String>,
    removed_metadata: bool,
    inlined_styles: bool,
    removed_unused_identifiers: bool,
    view_box: [i32; 4],
    coordinate_scale: i32,
}

fn serialize_element(
    node: Node<'_, '_>,
    is_root: bool,
    parent_name: Option<&str>,
    state: &mut SanitizeState<'_>,
) -> Result<Option<String>, ImportError> {
    let name = node.tag_name().name();
    if matches!(name, "title" | "desc" | "style") {
        return Ok(None);
    }
    if !ALLOWED_ELEMENTS.contains(&name) {
        return Err(ImportError::new(format!(
            "provider icon element '{name}' is not allowed"
        )));
    }
    if node.tag_name().namespace() != Some(SVG_NAMESPACE) {
        return Err(ImportError::new(
            "provider icon contains a foreign XML namespace",
        ));
    }
    if name == "svg" && !is_root {
        return Err(ImportError::new("nested SVG elements are not allowed"));
    }
    if name == "defs" && parent_name != Some("svg") {
        return Err(ImportError::new("defs must be a direct child of SVG"));
    }
    if matches!(
        name,
        "linearGradient" | "radialGradient" | "clipPath" | "mask"
    ) && parent_name != Some("defs")
    {
        return Err(ImportError::new(
            "gradients must be direct children of defs",
        ));
    }
    if name == "stop" && !matches!(parent_name, Some("linearGradient" | "radialGradient")) {
        return Err(ImportError::new(
            "gradient stops must be direct children of gradients",
        ));
    }
    if parent_name == Some("defs")
        && !matches!(
            name,
            "linearGradient" | "radialGradient" | "clipPath" | "mask"
        )
    {
        return Err(ImportError::new(
            "defs may contain only gradients, clip paths, or masks",
        ));
    }

    let mut attributes = BTreeMap::new();
    for attribute in node.attributes() {
        let is_xlink_href = attribute.name() == "href"
            && attribute
                .namespace()
                .is_some_and(|namespace| namespace == XLINK_NAMESPACE);
        if attribute.namespace().is_some() && !is_xlink_href {
            return Err(ImportError::new(
                "provider icon contains a namespaced attribute",
            ));
        }
        let attribute_name = if is_xlink_href {
            "href"
        } else {
            attribute.name()
        };
        if attribute_name.starts_with("on") {
            return Err(ImportError::new(
                "provider icon contains an event handler attribute",
            ));
        }
        match attribute_name {
            "version" | "data-name" | "xviewBox" => {
                state.removed_metadata = true;
                continue;
            }
            "viewBox" if is_root => {
                attributes.insert(
                    "viewBox".to_owned(),
                    state
                        .view_box
                        .iter()
                        .map(i32::to_string)
                        .collect::<Vec<_>>()
                        .join(" "),
                );
                continue;
            }
            "class" => {
                let fill = match state.styles.get(attribute.value()) {
                    Some(fill) => fill,
                    None => {
                        return Err(ImportError::new(
                            "provider icon references an unknown stylesheet class",
                        ));
                    }
                };
                let fill = if let Some(identifier) = local_url_reference(fill) {
                    let mapped = state.identifiers.get(identifier).ok_or_else(|| {
                        ImportError::new("provider icon references an undeclared identifier")
                    })?;
                    format!("url(#{mapped})")
                } else {
                    fill.clone()
                };
                attributes.insert("fill".to_owned(), fill);
                continue;
            }
            "style" => {
                if attribute.value().trim() != "isolation: isolate" {
                    return Err(ImportError::new(
                        "provider icon contains an unsupported inline style",
                    ));
                }
                attributes.insert("isolation".to_owned(), "isolate".to_owned());
                state.inlined_styles = true;
                continue;
            }
            "id" => {
                if let Some(identifier) = state.identifiers.get(attribute.value()) {
                    if !matches!(
                        name,
                        "linearGradient" | "radialGradient" | "clipPath" | "mask"
                    ) {
                        return Err(ImportError::new(
                            "only local SVG resources may retain provider icon identifiers",
                        ));
                    }
                    attributes.insert("id".to_owned(), identifier.clone());
                } else {
                    state.removed_unused_identifiers = true;
                }
                continue;
            }
            _ => {}
        }
        if !ALLOWED_ATTRIBUTES.contains(&attribute_name) {
            return Err(ImportError::new(format!(
                "provider icon attribute '{attribute_name}' is not allowed"
            )));
        }
        let value = if let Some(identifier) = local_url_reference(attribute.value()) {
            let mapped = match state.identifiers.get(identifier) {
                Some(mapped) => mapped,
                None => {
                    return Err(ImportError::new(
                        "provider icon references an undeclared identifier",
                    ));
                }
            };
            if !matches!(attribute_name, "fill" | "stroke" | "clip-path" | "mask") {
                return Err(ImportError::new(
                    "local URL references are not allowed for this attribute",
                ));
            }
            format!("url(#{mapped})")
        } else if attribute_name == "href" {
            let Some(identifier) = fragment_reference(attribute.value()) else {
                return Err(ImportError::new(
                    "provider icon href must be a local fragment",
                ));
            };
            if !matches!(name, "linearGradient" | "radialGradient") {
                return Err(ImportError::new(
                    "provider icon href is allowed only for gradients",
                ));
            }
            let mapped = state.identifiers.get(identifier).ok_or_else(|| {
                ImportError::new("provider icon references an undeclared identifier")
            })?;
            format!("#{mapped}")
        } else {
            validate_safe_value(attribute.value())?;
            attribute.value().to_owned()
        };
        if attributes
            .insert(attribute_name.to_owned(), value)
            .is_some()
        {
            return Err(ImportError::new(
                "provider icon declares a duplicate effective attribute",
            ));
        }
    }
    if is_root && !attributes.contains_key("viewBox") {
        attributes.insert(
            "viewBox".to_owned(),
            state
                .view_box
                .iter()
                .map(i32::to_string)
                .collect::<Vec<_>>()
                .join(" "),
        );
    }

    let mut definitions = String::new();
    let mut children = String::new();
    for child in node.children() {
        match child.node_type() {
            NodeType::Element => {
                if let Some(serialized) = serialize_element(child, false, Some(name), state)? {
                    if is_root && child.tag_name().name() == "defs" {
                        definitions.push_str(&serialized);
                    } else {
                        children.push_str(&serialized);
                    }
                }
            }
            NodeType::Text if is_ignorable_text(child.text().unwrap_or_default()) => {
                if child
                    .text()
                    .is_some_and(|text| text.contains('\u{200b}') || text.contains('\u{feff}'))
                {
                    state.removed_metadata = true;
                }
            }
            NodeType::Comment | NodeType::PI => state.removed_metadata = true,
            _ => {
                return Err(ImportError::new(
                    "provider icon contains unsupported visible text or XML content",
                ));
            }
        }
    }
    if name == "defs" && children.is_empty() {
        return Ok(None);
    }
    if is_root && state.coordinate_scale != 1 && !children.is_empty() {
        children = format!(
            "{definitions}<g transform=\"scale({})\">{children}</g>",
            state.coordinate_scale,
        );
    } else if is_root && !definitions.is_empty() {
        children = format!("{definitions}{children}");
    }

    let mut output = String::new();
    output.push('<');
    output.push_str(name);
    if is_root {
        output.push_str(" xmlns=\"http://www.w3.org/2000/svg\"");
    }
    for (name, value) in attributes {
        output.push(' ');
        output.push_str(&name);
        output.push_str("=\"");
        output.push_str(&escape_xml_attribute(&value));
        output.push('"');
    }
    if children.is_empty() {
        output.push_str("/>");
    } else {
        output.push('>');
        output.push_str(&children);
        output.push_str("</");
        output.push_str(name);
        output.push('>');
    }
    Ok(Some(output))
}

fn is_ignorable_text(value: &str) -> bool {
    value
        .chars()
        .all(|character| character.is_whitespace() || matches!(character, '\u{200b}' | '\u{feff}'))
}

fn validate_safe_value(value: &str) -> Result<(), ImportError> {
    let lowercase = value.to_ascii_lowercase();
    if lowercase.contains("url(")
        || lowercase.contains("javascript:")
        || lowercase.contains("data:")
        || lowercase.contains("http://")
        || lowercase.contains("https://")
        || lowercase.contains("//")
    {
        return Err(ImportError::new(
            "provider icon contains an external or executable reference",
        ));
    }
    Ok(())
}

fn escape_xml_attribute(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn write_pack_directory(
    root: &Path,
    manifest: &[u8],
    notice: &[u8],
    assets: &[(String, String)],
) -> Result<(), ImportError> {
    if fs::create_dir(root.join("assets")).is_err() {
        return Err(ImportError::new(
            "cannot create provider pack asset directory",
        ));
    }
    write_new_file(&root.join("manifest.json"), manifest)?;
    write_new_file(&root.join("NOTICE.md"), notice)?;
    for (relative_path, svg) in assets {
        write_new_file(&root.join(relative_path), svg.as_bytes())?;
    }
    Ok(())
}

fn write_new_file(path: &Path, contents: &[u8]) -> Result<(), ImportError> {
    let mut file = match OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(file) => file,
        Err(_) => return Err(ImportError::new("cannot create provider pack file")),
    };
    if file.write_all(contents).is_err() || file.sync_all().is_err() {
        return Err(ImportError::new("cannot write provider pack file"));
    }
    Ok(())
}

fn create_temporary_directory(parent: &Path) -> Result<PathBuf, ImportError> {
    for attempt in 0..128_u8 {
        let path = parent.join(format!(
            ".stack-provider-pack-{}-{attempt}",
            std::process::id()
        ));
        match fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => {
                return Err(ImportError::new(format!(
                    "cannot create temporary provider pack directory: {}",
                    stable_io_error(error.kind())
                )));
            }
        }
    }
    Err(ImportError::new(
        "cannot reserve a temporary provider pack directory",
    ))
}

fn render_notice(manifest: &ProviderPack) -> String {
    let mut notice = format!(
        "# Stack provider icon pack notice\n\nProvider: {} (`{}`)\n\n{}\n\n{}\n\n{}\n\nThis local pack was created from archives selected by the user. Stack does not redistribute these asset bytes. Use and distribute generated diagrams only as permitted by the linked provider and brand terms.\n\n## Sources\n",
        manifest.provider.name,
        manifest.provider.id,
        manifest.notice.attribution,
        manifest.notice.terms_summary,
        manifest.notice.non_endorsement,
    );
    render_notice_source(&mut notice, "primary", &manifest.source);
    for source in &manifest.additional_sources {
        render_notice_source(&mut notice, &source.id, &source.source);
    }
    notice.push_str("\n## Icons\n");
    for icon in &manifest.icons {
        let source_id = icon.asset.source_id.as_deref().unwrap_or("primary");
        let _ = write!(
            notice,
            "\n- `{}`: {} (source `{source_id}`)",
            icon.id, icon.product_name
        );
        if let Some(url) = &icon.brand_source_url {
            let _ = write!(notice, "; brand source <{url}>");
        }
        if let Some(url) = &icon.brand_guidelines_url {
            let _ = write!(notice, "; brand guidelines <{url}>");
        }
        notice.push('\n');
    }
    notice
}

fn render_notice_source(notice: &mut String, id: &str, source: &ProviderPackSource) {
    let _ = write!(
        notice,
        "\n### `{id}`\n\n- Source: <{}>\n- Official archive: <{}>\n- Release: {}\n- Archive SHA-256: `{}`\n- Terms: <{}>\n- Terms reviewed: {} (review again after {})\n",
        source.page_url,
        source.archive_url,
        source.release,
        source.archive_sha256,
        source.terms_url,
        source.terms_reviewed_at,
        source.review_after,
    );
}

fn sha256(bytes: &[u8]) -> String {
    let mut output = String::from("sha256:");
    for byte in Sha256::digest(bytes) {
        let _ = fmt::Write::write_fmt(&mut output, format_args!("{byte:02x}"));
    }
    output
}

fn stable_io_error(kind: std::io::ErrorKind) -> &'static str {
    match kind {
        std::io::ErrorKind::NotFound => "file not found",
        std::io::ErrorKind::PermissionDenied => "permission denied",
        std::io::ErrorKind::InvalidData => "invalid data",
        _ => "I/O error",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider_catalog::CatalogIcon;
    use zip::write::SimpleFileOptions;

    fn temporary_root(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!("stack-cli-provider-{label}-{}", std::process::id()))
    }

    fn zip_with_entry(name: &str, contents: &[u8]) -> Vec<u8> {
        let cursor = Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(cursor);
        assert!(
            writer
                .start_file(name, SimpleFileOptions::default())
                .is_ok()
        );
        assert!(writer.write_all(contents).is_ok());
        let result = writer.finish();
        assert!(result.is_ok());
        result.map(Cursor::into_inner).unwrap_or_default()
    }

    fn zip_with_directory(name: &str) -> Vec<u8> {
        let cursor = Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(cursor);
        assert!(
            writer
                .add_directory(name, SimpleFileOptions::default())
                .is_ok()
        );
        let result = writer.finish();
        assert!(result.is_ok());
        result.map(Cursor::into_inner).unwrap_or_default()
    }

    fn assert_svg_error(source: &[u8], expected: &str) {
        let error = sanitize_svg(source, "example", "fixture").err();
        assert!(
            error.is_some_and(|error| error.to_string().contains(expected)),
            "expected SVG error containing: {expected}"
        );
    }

    fn read_test_icon(archive: Vec<u8>, entry_path: &str) -> Result<Vec<u8>, ImportError> {
        let mut archive = match ZipArchive::new(Cursor::new(archive)) {
            Ok(archive) => archive,
            Err(_) => return Err(ImportError::new("invalid test archive")),
        };
        read_icon_entry(&mut archive, entry_path)
    }

    fn test_profile(archive_sha256: &str, archive_path: &str) -> Result<ProviderCatalog, String> {
        let mut profile = provider_catalog("aws")?;
        profile.provider.id = "example".to_owned();
        profile.provider.name = "Example Cloud".to_owned();
        profile.source.page_url = "https://example.com/icons".to_owned();
        profile.source.archive_url = "https://example.com/icons.zip".to_owned();
        profile.source.archive_sha256 = archive_sha256.to_owned();
        profile.source.release = "fixture-1".to_owned();
        profile.source.terms_url = "https://example.com/terms".to_owned();
        profile.source.copyright = "Copyright Example Cloud".to_owned();
        profile.source.license_id = "LicenseRef-Example-Icons".to_owned();
        profile.additional_sources.clear();
        profile.notice.attribution = "Example Cloud owns the icons.".to_owned();
        profile.notice.terms_summary = "Architecture diagram use only.".to_owned();
        profile.notice.non_endorsement = "Example Cloud does not endorse Stack.".to_owned();
        profile.icons = vec![CatalogIcon {
            id: "example:storage".to_owned(),
            subject: "Object storage service".to_owned(),
            product_name: "Example Storage".to_owned(),
            brand_source_url: None,
            brand_guidelines_url: None,
            recommended_node_kind: ProviderNodeKind::Storage,
            category: "Storage".to_owned(),
            source_id: None,
            archive_path: archive_path.to_owned(),
        }];
        Ok(profile)
    }

    fn import_test_profile(
        profile: Result<ProviderCatalog, String>,
        archive_path: &Path,
        output_path: &Path,
    ) -> Result<ImportSummary, ImportError> {
        import_profile(
            profile.map_err(ImportError::new)?,
            archive_path,
            &BTreeMap::new(),
            output_path,
        )
    }

    #[test]
    fn audited_provider_profiles_have_expected_coverage() {
        assert_eq!(
            provider_catalog("aws").map(|profile| profile.icons.len()),
            Ok(305)
        );
        assert_eq!(
            provider_catalog("gcp").map(|profile| profile.icons.len()),
            Ok(45)
        );
        assert_eq!(
            provider_catalog("azure").map(|profile| profile.icons.len()),
            Ok(639)
        );
        assert_eq!(
            provider_catalog("simple-icons").map(|profile| profile.icons.len()),
            Ok(62)
        );
        assert!(provider_catalog("unknown").is_err());
    }

    #[test]
    fn sanitizer_preserves_colors_and_namespaces_gradients() {
        let source = br##"<?xml version="1.0"?><svg xmlns="http://www.w3.org/2000/svg" version="1.1" viewBox="0 0 18 18"><defs><linearGradient id="paint" x1="0" y1="0" x2="18" y2="18"><stop offset="0" stop-color="#37c2b1"/><stop offset="1" stop-color="#258277"/></linearGradient></defs><title>Storage</title><path id="unused" fill="url(#paint)" d="M0 0h18v18H0z"/></svg>"##;
        let processed = sanitize_svg(source, "azure", "storage-accounts");
        assert!(processed.is_ok());
        let Ok(processed) = processed else {
            return;
        };
        assert_eq!(processed.view_box, [0, 0, 18, 18]);
        assert!(processed.svg.contains("#37c2b1"));
        assert!(processed.svg.contains("#258277"));
        assert!(
            processed
                .svg
                .contains("id=\"stack-azure-storage-accounts-gradient-0\"")
        );
        assert!(
            processed
                .svg
                .contains("fill=\"url(#stack-azure-storage-accounts-gradient-0)\"")
        );
        assert!(!processed.svg.contains("<title"));
        assert!(!processed.svg.contains("id=\"unused\""));
        assert!(
            processed
                .transformations
                .contains(&ProviderPackTransformation::NamespaceIdentifiers)
        );
    }

    #[test]
    fn sanitizer_inlines_the_audited_google_fill_classes() {
        let source = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512"><defs><style>.st0 { fill: none; } .st1 { fill: #4285f4; }</style></defs><rect class="st0" width="512" height="512"/><path class="st1" d="M1 1h10v10z"/></svg>"##;
        let processed = sanitize_svg(source, "gcp", "cloud-run");
        assert!(processed.is_ok());
        let Ok(processed) = processed else {
            return;
        };
        assert!(processed.svg.contains("fill=\"none\""));
        assert!(processed.svg.contains("fill=\"#4285f4\""));
        assert!(!processed.svg.contains("style"));
        assert!(!processed.svg.contains("class"));
        assert!(
            processed
                .transformations
                .contains(&ProviderPackTransformation::InlineStyles)
        );
    }

    #[test]
    fn sanitizer_rejects_active_or_external_svg_content() {
        for source in [
            br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><script>alert(1)</script></svg>"#.as_slice(),
            br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path onload="alert(1)" d="M0 0"/></svg>"#.as_slice(),
            br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="url(https://example.com/a.svg#x)" d="M0 0"/></svg>"#.as_slice(),
            br#"<!DOCTYPE svg><svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"/>"#.as_slice(),
        ] {
            assert!(sanitize_svg(source, "example", "unsafe").is_err());
        }
    }

    #[test]
    fn sanitizer_rejects_invalid_xml_roots_and_view_boxes() {
        for (source, expected) in [
            (b"\xff".as_slice(), "not valid UTF-8"),
            (
                b"<?xml version=\"1.0\"".as_slice(),
                "incomplete XML declaration",
            ),
            (b"<svg".as_slice(), "not well-formed SVG"),
            (
                br#"<g xmlns="http://www.w3.org/2000/svg"/>"#.as_slice(),
                "canonical SVG namespace",
            ),
            (
                br#"<svg viewBox="0 0 24 24"/>"#.as_slice(),
                "canonical SVG namespace",
            ),
            (
                br#"<svg xmlns="http://www.w3.org/2000/svg"/>"#.as_slice(),
                "missing viewBox",
            ),
            (
                br#"<?xml-stylesheet href="https://example.com/style.css"?><svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"/>"#.as_slice(),
                "forbidden XML declaration",
            ),
            (
                br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 wide 24"/>"#.as_slice(),
                "finite decimals",
            ),
            (
                br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0,,0,24,0"/>"#.as_slice(),
                "positive dimensions",
            ),
            (
                br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24"/>"#.as_slice(),
                "four finite decimal numbers",
            ),
        ] {
            assert_svg_error(source, expected);
        }

        for value in [
            "999999999999999999999999",
            "9223372036854775807",
            "2147483648",
        ] {
            let decimal_places = u32::from(value == "9223372036854775807");
            assert!(scaled_decimal(value, decimal_places).is_err());
        }
    }

    #[test]
    fn sanitizer_rejects_unsupported_stylesheets() {
        for (style, expected) in [
            ("", "stylesheet is empty"),
            ("bad", "invalid syntax"),
            ("path{fill:red}", "selector is not a class"),
            (".!{fill:red}", "invalid class name"),
            (".a{fill}", "declaration is invalid"),
            (".a{stroke:red}", "one fill property"),
            (".a{fill:red;fill:blue}", "one fill property"),
            (".a{}", "does not define fill"),
            (".a{fill:red}.a{fill:blue}", "duplicate class"),
            (".a{fill:javascript:alert(1)}", "external or executable"),
        ] {
            let source = format!(
                "<svg xmlns=\"{SVG_NAMESPACE}\" viewBox=\"0 0 24 24\"><defs><style>{style}</style></defs><path class=\"a\" d=\"M0 0\"/></svg>"
            );
            assert_svg_error(source.as_bytes(), expected);
        }
        assert_svg_error(
            format!(
                "<svg xmlns=\"{SVG_NAMESPACE}\" viewBox=\"0 0 24 24\"><defs><style/></defs></svg>"
            )
            .as_bytes(),
            "stylesheet is empty",
        );
    }

    #[test]
    fn sanitizer_rejects_unsafe_structure_attributes_and_identifiers() {
        for (body, expected) in [
            (
                "<defs><linearGradient id=\"a\"/><linearGradient id=\"a\"/></defs><path fill=\"url(#a)\"/>",
                "duplicate identifier",
            ),
            (
                "<path fill=\"url(#missing)\" d=\"M0 0\"/>",
                "undeclared or duplicate identifier",
            ),
            (
                "<foreign:path xmlns:foreign=\"https://example.com/ns\"/>",
                "foreign XML namespace",
            ),
            ("<svg viewBox=\"0 0 1 1\"/>", "nested SVG"),
            ("<g><defs/></g>", "defs must be a direct child"),
            ("<linearGradient/>", "gradients must be direct children"),
            ("<g><stop/></g>", "gradient stops must be direct children"),
            (
                "<defs><path d=\"M0 0\"/></defs>",
                "defs may contain only gradients",
            ),
            ("<path xml:lang=\"en\" d=\"M0 0\"/>", "namespaced attribute"),
            (
                "<path class=\"missing\" d=\"M0 0\"/>",
                "unknown stylesheet class",
            ),
            (
                "<path id=\"shape\" d=\"M0 0\"/><path fill=\"url(#shape)\" d=\"M0 0\"/>",
                "only local SVG resources may retain",
            ),
            (
                "<path unknown=\"value\" d=\"M0 0\"/>",
                "attribute 'unknown' is not allowed",
            ),
            (
                "<defs><linearGradient id=\"paint\"/></defs><path d=\"url(#paint)\"/>",
                "not allowed for this attribute",
            ),
            ("<path>visible</path>", "unsupported visible text"),
            (
                "<path fill=\"https://example.com/icon.svg\" d=\"M0 0\"/>",
                "external or executable",
            ),
        ] {
            let source =
                format!("<svg xmlns=\"{SVG_NAMESPACE}\" viewBox=\"0 0 24 24\">{body}</svg>");
            assert_svg_error(source.as_bytes(), expected);
        }

        let duplicate_fill = format!(
            "<svg xmlns=\"{SVG_NAMESPACE}\" viewBox=\"0 0 24 24\"><defs><style>.a{{fill:red}}</style></defs><path class=\"a\" fill=\"blue\" d=\"M0 0\"/></svg>"
        );
        assert_svg_error(
            &duplicate_fill.into_bytes(),
            "duplicate effective attribute",
        );
    }

    #[test]
    fn sanitizer_preserves_local_resources_and_scales_decimal_view_boxes() {
        let source = [
            r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" data-name="Layer" viewBox="0 0 52.434 44.65"><defs><style>.paint{fill:url(#derived)}</style><linearGradient id="base"><stop offset="0" stop-color="#000"/><stop offset="1" stop-color="#fff"/></linearGradient><linearGradient id="derived" xlink:href="#base"/><clipPath id="clip"><rect width="52.434" height="44.65"/></clipPath><mask id="mask" maskUnits="userSpaceOnUse"><rect width="52.434" height="44.65" fill="#fff"/></mask></defs><path class="paint" clip-path="url(#clip)" mask="url(#mask)" style="isolation: isolate" d="M0 0h52.434v44.65H0z"/>"##,
            "\u{200b}",
            "</svg>",
        ]
        .concat();
        let processed = sanitize_svg(source.as_bytes(), "azure", "fixture");
        assert!(processed.is_ok());
        let Some(processed) = processed.ok() else {
            return;
        };
        assert_eq!(processed.view_box, [0, 0, 52_434, 44_650]);
        assert!(processed.svg.contains("viewBox=\"0 0 52434 44650\""));
        assert!(processed.svg.contains("<g transform=\"scale(1000)\">"));
        assert!(
            processed
                .svg
                .contains("href=\"#stack-azure-fixture-gradient-")
        );
        assert!(
            processed
                .svg
                .contains("clip-path=\"url(#stack-azure-fixture-gradient-")
        );
        assert!(
            processed
                .svg
                .contains("mask=\"url(#stack-azure-fixture-gradient-")
        );
        assert!(processed.svg.contains("isolation=\"isolate\""));
        assert!(!processed.svg.contains("data-name"));
        assert!(
            processed
                .transformations
                .contains(&ProviderPackTransformation::ScaleViewBoxToIntegers)
        );
    }

    #[test]
    fn sanitizer_removes_empty_definitions_and_escapes_attributes() {
        let source = format!(
            "<svg xmlns=\"{SVG_NAMESPACE}\" viewBox=\"0, 0, 24, 24\"><defs><title>metadata</title></defs><path aria-hidden=\"a&amp;b&lt;c&gt;d&quot;e\" d=\"M0 0\"/></svg>"
        );
        let processed = sanitize_svg(source.as_bytes(), "example", "escape");
        assert!(processed.is_ok());
        let Some(processed) = processed.ok() else {
            return;
        };
        assert!(!processed.svg.contains("<defs"));
        assert!(processed.svg.contains("a&amp;b&lt;c&gt;d&quot;e"));
        assert!(
            processed
                .transformations
                .contains(&ProviderPackTransformation::RemoveMetadata)
        );
    }

    #[test]
    fn archive_entry_checks_reject_missing_directories_and_large_icons() {
        let archive = zip_with_entry("actual.svg", b"svg");
        assert!(
            read_test_icon(archive, "missing.svg")
                .err()
                .is_some_and(|error| error.to_string().contains("missing required icon"))
        );

        let archive = zip_with_directory("icons/");
        assert!(
            read_test_icon(archive, "icons/")
                .err()
                .is_some_and(|error| error.to_string().contains("not a regular file"))
        );

        let archive = zip_with_entry("large.svg", &vec![b'x'; MAX_ICON_BYTES as usize + 1]);
        assert!(
            read_test_icon(archive, "large.svg")
                .err()
                .is_some_and(|error| error.to_string().contains("1 MiB limit"))
        );
    }

    #[test]
    fn local_import_rejects_invalid_archive_inputs_and_output_parent() {
        let root = temporary_root("invalid-inputs");
        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_file(&root);
        assert!(fs::create_dir(&root).is_ok());
        let output_path = root.join("pack");

        assert!(
            import_provider_pack(
                "unknown",
                &root.join("missing.zip"),
                &BTreeMap::new(),
                &output_path,
            )
            .err()
            .is_some_and(|error| error.to_string().contains("unknown provider"))
        );
        assert!(
            import_provider_pack(
                "aws",
                &root.join("missing.zip"),
                &BTreeMap::new(),
                &output_path,
            )
            .err()
            .is_some_and(|error| error.to_string().contains("file not found"))
        );
        assert!(
            import_test_profile(test_profile("sha256:none", "icon.svg"), &root, &output_path)
                .err()
                .is_some_and(|error| error.to_string().contains("not a file"))
        );

        let large_archive_path = root.join("large.zip");
        let large_archive = fs::File::create(&large_archive_path);
        assert!(large_archive.is_ok());
        if let Ok(large_archive) = large_archive {
            assert!(large_archive.set_len(MAX_ARCHIVE_BYTES + 1).is_ok());
        }
        assert!(
            import_test_profile(
                test_profile("sha256:none", "icon.svg"),
                &large_archive_path,
                &output_path,
            )
            .err()
            .is_some_and(|error| error.to_string().contains("32 MiB limit"))
        );

        let invalid_archive_path = root.join("invalid.zip");
        let invalid_archive = b"not a zip archive";
        assert!(fs::write(&invalid_archive_path, invalid_archive).is_ok());
        let invalid_digest = sha256(invalid_archive);
        assert!(
            import_test_profile(
                test_profile(&invalid_digest, "icon.svg"),
                &invalid_archive_path,
                &output_path,
            )
            .err()
            .is_some_and(|error| error.to_string().contains("not a supported ZIP"))
        );

        let valid_archive_path = root.join("valid.zip");
        let valid_archive = zip_with_entry(
            "icon.svg",
            format!("<svg xmlns=\"{SVG_NAMESPACE}\" viewBox=\"0 0 24 24\"/>").as_bytes(),
        );
        assert!(fs::write(&valid_archive_path, &valid_archive).is_ok());
        let parent_file = root.join("not-a-directory");
        assert!(fs::write(&parent_file, b"file").is_ok());
        assert!(
            import_test_profile(
                test_profile(&sha256(&valid_archive), "icon.svg"),
                &valid_archive_path,
                &parent_file.join("pack"),
            )
            .err()
            .is_some_and(|error| error.to_string().contains("not a directory"))
        );
        assert!(fs::remove_dir_all(&root).is_ok());
    }

    #[test]
    fn pack_writes_and_temporary_directories_fail_closed() {
        let root = temporary_root("write-failures");
        let _ = fs::remove_dir_all(&root);
        assert!(fs::create_dir(&root).is_ok());

        assert!(fs::create_dir(root.join("assets")).is_ok());
        assert!(
            write_pack_directory(&root, b"{}", b"notice", &[])
                .err()
                .is_some_and(|error| error.to_string().contains("asset directory"))
        );
        let existing = root.join("existing");
        assert!(fs::write(&existing, b"existing").is_ok());
        assert!(
            write_new_file(&existing, b"replacement")
                .err()
                .is_some_and(|error| error.to_string().contains("cannot create"))
        );
        assert!(
            create_temporary_directory(&root.join("missing"))
                .err()
                .is_some_and(|error| error.to_string().contains("cannot create temporary"))
        );

        let collision_root = root.join("collisions");
        assert!(fs::create_dir(&collision_root).is_ok());
        for attempt in 0..128_u8 {
            assert!(
                fs::create_dir(collision_root.join(format!(
                    ".stack-provider-pack-{}-{attempt}",
                    std::process::id()
                )))
                .is_ok()
            );
        }
        assert!(
            create_temporary_directory(&collision_root)
                .err()
                .is_some_and(|error| error.to_string().contains("cannot reserve"))
        );
        assert_eq!(
            stable_io_error(std::io::ErrorKind::PermissionDenied),
            "permission denied"
        );
        assert_eq!(
            stable_io_error(std::io::ErrorKind::InvalidData),
            "invalid data"
        );
        assert_eq!(stable_io_error(std::io::ErrorKind::Other), "I/O error");
        assert!(fs::remove_dir_all(&root).is_ok());
    }

    #[test]
    fn local_import_writes_manifest_notice_and_sanitized_asset_atomically() {
        let root = temporary_root("success");
        let archive_path = root.with_extension("zip");
        let output_path = root.join("pack");
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="#123456" d="M0 0h24v24H0z"/></svg>"##;
        let archive = zip_with_entry("icons/storage.svg", svg);
        assert!(fs::write(&archive_path, &archive).is_ok());
        assert!(fs::create_dir(&root).is_ok());
        let digest = sha256(&archive);
        let imported = import_test_profile(
            test_profile(&digest, "icons/storage.svg"),
            &archive_path,
            &output_path,
        );
        assert!(imported.is_ok());
        let Ok(imported) = imported else {
            return;
        };
        assert_eq!(imported.icon_count, 1);
        let manifest = fs::read_to_string(&imported.manifest_path);
        assert!(manifest.is_ok());
        let Some(manifest): Option<ProviderPack> = manifest
            .ok()
            .and_then(|value| serde_json::from_str(&value).ok())
        else {
            let _ = fs::remove_dir_all(&root);
            let _ = fs::remove_file(&archive_path);
            return;
        };
        assert_eq!(manifest.provider.id, "example");
        assert_eq!(manifest.icons[0].id, "example:storage");
        assert_eq!(manifest.icons[0].asset.original_sha256, sha256(svg));
        assert!(output_path.join("assets/storage.svg").is_file());
        assert!(
            fs::read_to_string(imported.notice_path)
                .unwrap_or_default()
                .contains("Stack does not redistribute these asset bytes")
        );
        assert!(fs::remove_dir_all(&root).is_ok());
        assert!(fs::remove_file(&archive_path).is_ok());
    }

    #[test]
    fn local_import_requires_and_records_every_declared_source() {
        let root = temporary_root("multiple-sources");
        let _ = fs::remove_dir_all(&root);
        assert!(fs::create_dir(&root).is_ok());
        let primary_path = root.join("primary.zip");
        let categories_path = root.join("categories.zip");
        let primary = zip_with_entry(
            "icons/storage.svg",
            format!("<svg xmlns=\"{SVG_NAMESPACE}\" viewBox=\"0 0 24 24\"/>").as_bytes(),
        );
        let categories = zip_with_entry(
            "icons/category.svg",
            format!("<svg xmlns=\"{SVG_NAMESPACE}\" viewBox=\"0 0 24 24\"/>").as_bytes(),
        );
        assert!(fs::write(&primary_path, &primary).is_ok());
        assert!(fs::write(&categories_path, &categories).is_ok());

        let Ok(mut profile) = test_profile(&sha256(&primary), "icons/storage.svg") else {
            let _ = fs::remove_dir_all(&root);
            return;
        };
        let Ok(gcp) = provider_catalog("gcp") else {
            let _ = fs::remove_dir_all(&root);
            return;
        };
        let Some(mut additional) = gcp.additional_sources.first().cloned() else {
            let _ = fs::remove_dir_all(&root);
            return;
        };
        additional.source.archive_sha256 = sha256(&categories);
        additional.source.archive_url = "https://example.com/categories.zip".to_owned();
        profile.additional_sources = vec![additional];
        profile.icons.push(CatalogIcon {
            id: "example:category".to_owned(),
            subject: "Example category".to_owned(),
            product_name: "Example Category".to_owned(),
            brand_source_url: None,
            brand_guidelines_url: None,
            recommended_node_kind: ProviderNodeKind::Service,
            category: "Category".to_owned(),
            source_id: Some("categories".to_owned()),
            archive_path: "icons/category.svg".to_owned(),
        });

        assert!(
            import_profile(
                profile.clone(),
                &primary_path,
                &BTreeMap::new(),
                &root.join("missing"),
            )
            .err()
            .is_some_and(|error| error.to_string().contains("missing local archive"))
        );
        let unexpected = BTreeMap::from([("other".to_owned(), categories_path.clone())]);
        assert!(
            import_profile(
                profile.clone(),
                &primary_path,
                &unexpected,
                &root.join("unexpected"),
            )
            .err()
            .is_some_and(|error| error.to_string().contains("does not declare"))
        );

        let sources = BTreeMap::from([("categories".to_owned(), categories_path)]);
        let imported = import_profile(profile, &primary_path, &sources, &root.join("pack"));
        assert!(imported.is_ok());
        let manifest = fs::read_to_string(root.join("pack/manifest.json")).unwrap_or_default();
        assert!(manifest.contains("\"schemaVersion\": \"1.1\""));
        assert!(manifest.contains("\"sourceId\": \"categories\""));
        let notice = fs::read_to_string(root.join("pack/NOTICE.md")).unwrap_or_default();
        assert!(notice.contains("### `primary`"));
        assert!(notice.contains("### `categories`"));
        assert!(notice.contains("source `categories`"));
        assert!(fs::remove_dir_all(&root).is_ok());
    }

    #[test]
    fn local_import_rejects_hash_mismatch_existing_output_and_unsafe_paths() {
        let root = temporary_root("failures");
        let archive_path = root.with_extension("zip");
        let output_path = root.join("pack");
        let archive = zip_with_entry("../unsafe.svg", b"not svg");
        assert!(fs::write(&archive_path, &archive).is_ok());
        assert!(fs::create_dir(&root).is_ok());

        let mismatch = import_test_profile(
            test_profile(
                "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "../unsafe.svg",
            ),
            &archive_path,
            &output_path,
        );
        assert!(
            mismatch
                .err()
                .is_some_and(|error| error.to_string().contains("archive hash does not match"))
        );

        let digest = sha256(&archive);
        let unsafe_path = import_test_profile(
            test_profile(&digest, "../unsafe.svg"),
            &archive_path,
            &output_path,
        );
        assert!(
            unsafe_path
                .err()
                .is_some_and(|error| error.to_string().contains("unsafe path"))
        );

        assert!(fs::create_dir(&output_path).is_ok());
        let existing = import_test_profile(
            test_profile(&digest, "../unsafe.svg"),
            &archive_path,
            &output_path,
        );
        assert!(
            existing
                .err()
                .is_some_and(|error| error.to_string().contains("already exists"))
        );
        assert!(fs::remove_dir_all(&root).is_ok());
        assert!(fs::remove_file(&archive_path).is_ok());
    }
}
