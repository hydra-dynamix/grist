//! Bounded, inert ZIP/ZIP64 and TAR inventory and traversal.

use crate::container::{
    ArchiveMemberMetadata, ContainerArtifactMode, ContainerClass, ContainerDecodeContext,
    ContainerDecodeFailure, ContainerDecoder, ContainerDecoderRegistry, ContainerMember,
    ContainerParseOptions, ContainerParseRequest, ContainerRecursor, ContainerRegistryError,
    ContainerTraversal, ContainerTraversalError, ContainerUnavailable, ContainerUnavailableKind,
    ContentAddressedArtifactSink, ParentRelativeLocator,
};
use crate::core::{
    ArtifactKind, BudgetProfile, BudgetSelection, Envelope, FormatHint, IndexPosition,
    LocationComponent, OperationControl, OperationKind, OperationStatus, ParserInfo, RequestId,
    SchemaVersion, SourceInfo,
};
use crate::document_graph::{
    DocumentGraph, DocumentGraphContext, DocumentKind, DocumentNode, DocumentNodeKind,
    GraphIdGenerator, ProjectionAddress, ToDocumentGraph, TransformError,
};
use crate::ingest::Ingestor;
use crate::security::{ArchiveEntryKind, ArchiveMemberDescriptor};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::{Cursor, Read};
use std::sync::Arc;

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

const PARSER: &str = "grist.archive";

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ArchiveFormat {
    Zip,
    Zip64,
    Tar,
}

impl ArchiveFormat {
    pub const fn container_id(self) -> &'static str {
        match self {
            Self::Zip | Self::Zip64 => "zip",
            Self::Tar => "tar",
        }
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArchiveOptions {
    pub artifact_mode: ContainerArtifactMode,
    pub parse_leaf_payloads: bool,
}

impl Default for ArchiveOptions {
    fn default() -> Self {
        Self {
            artifact_mode: ContainerArtifactMode::InventoryOnly,
            parse_leaf_payloads: false,
        }
    }
}

impl crate::core::FormatOptions for ArchiveOptions {
    const FORMAT: &'static str = "archive";
}

impl ArchiveOptions {
    fn container_options(&self) -> ContainerParseOptions {
        ContainerParseOptions {
            artifact_mode: self.artifact_mode,
            parse_leaf_payloads: self.parse_leaf_payloads,
        }
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ArchiveDocument {
    pub schema_version: String,
    pub format: ArchiveFormat,
    pub traversal: ContainerTraversal,
}

pub type ArchiveEnvelope = Envelope<ArchiveDocument>;

#[derive(Debug, thiserror::Error)]
pub enum ArchiveParseError {
    #[error("unsupported archive format {0}")]
    UnsupportedFormat(String),
    #[error(transparent)]
    Registry(#[from] ContainerRegistryError),
    #[error(transparent)]
    Traversal(#[from] ContainerTraversalError),
    #[error("parser registry initialization failed: {0}")]
    Ingestor(String),
    #[error(transparent)]
    Serialization(#[from] serde_json::Error),
}

pub fn parser_info() -> ParserInfo {
    ParserInfo::new(PARSER)
        .with_implementation("zip + tar", "4.6.1/0.4.46")
        .with_specification_version("ZIP APPNOTE 6.3.10; POSIX.1-2001 pax")
        .with_feature("archives")
}

pub fn archive_format(bytes: &[u8], hint: Option<&str>) -> Option<ArchiveFormat> {
    match hint.map(normalize_format).as_deref() {
        Some("zip") | Some("zip64") => Some(if is_zip64(bytes) {
            ArchiveFormat::Zip64
        } else {
            ArchiveFormat::Zip
        }),
        Some("tar") => Some(ArchiveFormat::Tar),
        Some(_) => None,
        None if is_zip(bytes) => Some(if is_zip64(bytes) {
            ArchiveFormat::Zip64
        } else {
            ArchiveFormat::Zip
        }),
        None if is_tar(bytes) => Some(ArchiveFormat::Tar),
        None => None,
    }
}

pub fn builtin_decoder_registry() -> Result<ContainerDecoderRegistry, ContainerRegistryError> {
    let mut registry = ContainerDecoderRegistry::new();
    registry.register(Arc::new(ZipDecoder))?;
    registry.register(Arc::new(TarDecoder))?;
    Ok(registry)
}

pub fn traverse_archive(
    request: ContainerParseRequest,
    sink: Option<&dyn ContentAddressedArtifactSink>,
) -> Result<ContainerTraversal, ArchiveParseError> {
    traverse_archive_controlled(request, sink, None)
}

fn traverse_archive_controlled(
    request: ContainerParseRequest,
    sink: Option<&dyn ContentAddressedArtifactSink>,
    control: Option<OperationControl>,
) -> Result<ContainerTraversal, ArchiveParseError> {
    if !matches!(
        normalize_format(&request.container_format).as_str(),
        "zip" | "tar"
    ) {
        return Err(ArchiveParseError::UnsupportedFormat(
            request.container_format.clone(),
        ));
    }
    let ingestor =
        Ingestor::builtin().map_err(|error| ArchiveParseError::Ingestor(error.to_string()))?;
    let decoders = builtin_decoder_registry()?;
    let recursor = ContainerRecursor::new(&ingestor, &decoders);
    Ok(match control {
        Some(control) => recursor.parse_with_control(request, sink, control)?,
        None => recursor.parse(request, sink)?,
    })
}

pub fn parse_archive(
    bytes: &[u8],
    source: SourceInfo,
    request_id: RequestId,
    format_hint: Option<&str>,
    options: &ArchiveOptions,
) -> Result<ArchiveEnvelope, ArchiveParseError> {
    let format = archive_format(bytes, format_hint).ok_or_else(|| {
        ArchiveParseError::UnsupportedFormat(format_hint.unwrap_or("auto").into())
    })?;
    let request = ContainerParseRequest::new(
        request_id,
        bytes.to_vec(),
        source.clone(),
        format.container_id(),
        options.container_options(),
        BudgetSelection::Profile(BudgetProfile::UntrustedServiceV1),
    );
    parse_archive_request(request, format, options, None)
}

pub(crate) fn parse_archive_request_with_control(
    request: ContainerParseRequest,
    format: ArchiveFormat,
    options: &ArchiveOptions,
    control: OperationControl,
) -> Result<ArchiveEnvelope, ArchiveParseError> {
    parse_archive_request(request, format, options, Some(control))
}

fn parse_archive_request(
    request: ContainerParseRequest,
    format: ArchiveFormat,
    options: &ArchiveOptions,
    control: Option<OperationControl>,
) -> Result<ArchiveEnvelope, ArchiveParseError> {
    let source = request.source.clone();
    let traversal = traverse_archive_controlled(request, None, control)?;
    let status = traversal.status;
    let identity = traversal.identity.clone();
    let diagnostics = traversal.diagnostics.clone();
    let document = ArchiveDocument {
        schema_version: SchemaVersion::ARCHIVE_V1.into(),
        format,
        traversal,
    };
    let digest = crate::core::options_digest(options)?;
    let envelope = match status {
        OperationStatus::Complete => Envelope::complete(
            OperationKind::Parse,
            ArtifactKind::Archive,
            source,
            parser_info(),
            digest,
            SchemaVersion::ARCHIVE_V1,
            document,
        ),
        OperationStatus::Partial => Envelope::partial(
            OperationKind::Parse,
            ArtifactKind::Archive,
            source,
            parser_info(),
            digest,
            SchemaVersion::ARCHIVE_V1,
            Some(document),
        ),
        terminal => {
            return Ok(Envelope::without_payload(
                OperationKind::Parse,
                ArtifactKind::Archive,
                terminal,
                source,
                parser_info(),
                digest,
                SchemaVersion::ARCHIVE_V1,
            )
            .expect("archive terminal envelope is valid")
            .with_identity(identity)
            .with_diagnostics(diagnostics));
        }
    };
    Ok(envelope
        .with_identity(identity)
        .with_diagnostics(diagnostics)
        .with_canonical_payload_identity()?)
}

#[derive(Debug)]
pub struct ZipDecoder;

impl ContainerDecoder for ZipDecoder {
    fn format(&self) -> &str {
        "zip"
    }

    fn class(&self) -> ContainerClass {
        ContainerClass::Archive
    }

    fn decode(
        &self,
        context: &ContainerDecodeContext<'_>,
    ) -> Result<Vec<ContainerMember>, ContainerDecodeFailure> {
        decode_zip(context)
    }
}

#[derive(Debug)]
pub struct TarDecoder;

impl ContainerDecoder for TarDecoder {
    fn format(&self) -> &str {
        "tar"
    }

    fn class(&self) -> ContainerClass {
        ContainerClass::Archive
    }

    fn decode(
        &self,
        context: &ContainerDecodeContext<'_>,
    ) -> Result<Vec<ContainerMember>, ContainerDecodeFailure> {
        decode_tar(context)
    }
}

#[derive(Clone)]
struct ZipEntryInfo {
    index: usize,
    path: String,
    metadata: ArchiveMemberMetadata,
    rejected: Option<(&'static str, String)>,
    unavailable: Option<ContainerUnavailable>,
}

fn zip_central_directory(
    bytes: &[u8],
    context: &ContainerDecodeContext<'_>,
) -> Result<Vec<ZipEntryInfo>, ContainerDecodeFailure> {
    let minimum = 22_usize;
    if bytes.len() < minimum {
        return Err(malformed("ZIP end-of-central-directory record is missing"));
    }
    let search_start = bytes.len().saturating_sub(minimum + 65_535);
    let eocd = (search_start..=bytes.len() - minimum)
        .rev()
        .find(|offset| bytes.get(*offset..offset.saturating_add(4)) == Some(b"PK\x05\x06"))
        .ok_or_else(|| malformed("ZIP end-of-central-directory record is missing"))?;

    let mut entry_count = u64::from(
        little_u16(bytes, eocd + 10)
            .ok_or_else(|| malformed("truncated ZIP end-of-central-directory record"))?,
    );
    let mut directory_offset = u64::from(
        little_u32(bytes, eocd + 16)
            .ok_or_else(|| malformed("truncated ZIP end-of-central-directory record"))?,
    );
    if entry_count == u64::from(u16::MAX) || directory_offset == u64::from(u32::MAX) {
        let locator = eocd
            .checked_sub(20)
            .filter(|offset| bytes.get(*offset..offset.saturating_add(4)) == Some(b"PK\x06\x07"))
            .ok_or_else(|| malformed("ZIP64 end-of-central-directory locator is missing"))?;
        let zip64_offset = usize::try_from(
            little_u64(bytes, locator + 8).ok_or_else(|| malformed("truncated ZIP64 locator"))?,
        )
        .map_err(|_| malformed("ZIP64 directory offset exceeds this platform's address space"))?;
        if bytes.get(zip64_offset..zip64_offset.saturating_add(4)) != Some(b"PK\x06\x06") {
            return Err(malformed(
                "ZIP64 end-of-central-directory record is missing",
            ));
        }
        entry_count = little_u64(bytes, zip64_offset + 32)
            .ok_or_else(|| malformed("truncated ZIP64 end-of-central-directory record"))?;
        directory_offset = little_u64(bytes, zip64_offset + 48)
            .ok_or_else(|| malformed("truncated ZIP64 end-of-central-directory record"))?;
    }

    context.check_archive_member_count(entry_count)?;
    let count = usize::try_from(entry_count)
        .map_err(|_| malformed("ZIP member count exceeds this platform's address space"))?;
    let mut offset = usize::try_from(directory_offset)
        .map_err(|_| malformed("ZIP directory offset exceeds this platform's address space"))?;
    let mut entries = Vec::with_capacity(count.min(1_000_000));
    for index in 0..count {
        if bytes.get(offset..offset.saturating_add(4)) != Some(b"PK\x01\x02") {
            return Err(malformed(format!(
                "ZIP central-directory entry {index} is missing or truncated"
            )));
        }
        let flags = little_u16(bytes, offset + 8)
            .ok_or_else(|| malformed("truncated ZIP central-directory flags"))?;
        let method = little_u16(bytes, offset + 10)
            .ok_or_else(|| malformed("truncated ZIP compression method"))?;
        let crc32 =
            little_u32(bytes, offset + 16).ok_or_else(|| malformed("truncated ZIP CRC-32"))?;
        let compressed32 = little_u32(bytes, offset + 20)
            .ok_or_else(|| malformed("truncated ZIP compressed size"))?;
        let uncompressed32 = little_u32(bytes, offset + 24)
            .ok_or_else(|| malformed("truncated ZIP uncompressed size"))?;
        let name_len = usize::from(
            little_u16(bytes, offset + 28)
                .ok_or_else(|| malformed("truncated ZIP member name length"))?,
        );
        let extra_len = usize::from(
            little_u16(bytes, offset + 30)
                .ok_or_else(|| malformed("truncated ZIP extra-field length"))?,
        );
        let comment_len = usize::from(
            little_u16(bytes, offset + 32)
                .ok_or_else(|| malformed("truncated ZIP comment length"))?,
        );
        let header_end = offset
            .checked_add(46)
            .ok_or_else(|| malformed("ZIP central-directory offset overflow"))?;
        let name_end = header_end
            .checked_add(name_len)
            .ok_or_else(|| malformed("ZIP member-name offset overflow"))?;
        let extra_end = name_end
            .checked_add(extra_len)
            .ok_or_else(|| malformed("ZIP extra-field offset overflow"))?;
        let next = extra_end
            .checked_add(comment_len)
            .ok_or_else(|| malformed("ZIP comment offset overflow"))?;
        let name = bytes
            .get(header_end..name_end)
            .ok_or_else(|| malformed("truncated ZIP member name"))?;
        let extra = bytes
            .get(name_end..extra_end)
            .ok_or_else(|| malformed("truncated ZIP extra field"))?;
        if next > bytes.len() {
            return Err(malformed("truncated ZIP central-directory entry"));
        }

        let needs_uncompressed = uncompressed32 == u32::MAX;
        let needs_compressed = compressed32 == u32::MAX;
        let (zip64_uncompressed, zip64_compressed, has_zip64) =
            zip64_sizes(extra, needs_uncompressed, needs_compressed)?;
        let uncompressed_size = if needs_uncompressed {
            zip64_uncompressed.ok_or_else(|| malformed("ZIP64 uncompressed size is missing"))?
        } else {
            u64::from(uncompressed32)
        };
        let compressed_size = if needs_compressed {
            zip64_compressed.ok_or_else(|| malformed("ZIP64 compressed size is missing"))?
        } else {
            u64::from(compressed32)
        };
        let path = String::from_utf8_lossy(name).replace('\u{5c}', "/");
        let made_by = *bytes
            .get(offset + 5)
            .ok_or_else(|| malformed("truncated ZIP creator field"))?;
        let external = little_u32(bytes, offset + 38)
            .ok_or_else(|| malformed("truncated ZIP external attributes"))?;
        let mode = (made_by == 3).then_some(external >> 16);
        let directory = path.ends_with('/') || external & 0x10 != 0;
        let entry_kind = zip_entry_kind(directory, false, mode);
        entries.push(ZipEntryInfo {
            index,
            path,
            metadata: ArchiveMemberMetadata {
                entry_kind,
                compression_method: zip_compression_name(method),
                compressed_size,
                uncompressed_size,
                crc32: Some(crc32),
                mode,
                uid: None,
                gid: None,
                modified_time: None,
                link_target: None,
                encrypted: flags & 1 != 0 || method == 99,
                zip64: has_zip64 || needs_compressed || needs_uncompressed,
            },
            rejected: None,
            unavailable: None,
        });
        offset = next;
    }
    Ok(entries)
}

fn zip64_sizes(
    extra: &[u8],
    needs_uncompressed: bool,
    needs_compressed: bool,
) -> Result<(Option<u64>, Option<u64>, bool), ContainerDecodeFailure> {
    let mut cursor = 0_usize;
    while cursor.saturating_add(4) <= extra.len() {
        let id = little_u16(extra, cursor).expect("bounded ZIP extra-field ID");
        let size =
            usize::from(little_u16(extra, cursor + 2).expect("bounded ZIP extra-field size"));
        let data_start = cursor + 4;
        let data_end = data_start
            .checked_add(size)
            .ok_or_else(|| malformed("ZIP extra-field size overflow"))?;
        let data = extra
            .get(data_start..data_end)
            .ok_or_else(|| malformed("truncated ZIP extra field"))?;
        if id == 0x0001 {
            let mut value_offset = 0_usize;
            let uncompressed = if needs_uncompressed {
                let value = little_u64(data, value_offset);
                value_offset += 8;
                value
            } else {
                None
            };
            let compressed = if needs_compressed {
                little_u64(data, value_offset)
            } else {
                None
            };
            return Ok((uncompressed, compressed, true));
        }
        cursor = data_end;
    }
    Ok((None, None, false))
}

fn zip_compression_name(method: u16) -> String {
    match method {
        0 => "stored".into(),
        8 => "deflated".into(),
        12 => "bzip2".into(),
        14 => "lzma".into(),
        93 => "zstd".into(),
        99 => "aes".into(),
        value => format!("method-{value}"),
    }
}

fn zip_compression_supported(method: &str) -> bool {
    matches!(method, "stored" | "deflated")
}

fn materialize_zip_member(
    context: &ContainerDecodeContext<'_>,
    archive: Option<&mut zip::ZipArchive<Cursor<&[u8]>>>,
    entry: &ZipEntryInfo,
    locator: ParentRelativeLocator,
) -> Result<ContainerMember, ContainerDecodeFailure> {
    context.check_member_capacity(entry.metadata.uncompressed_size)?;
    Ok(match read_zip_member_bytes(archive, entry) {
        Ok(bytes) => {
            let nested = nested_archive_format(&bytes);
            let mut member = ContainerMember::available(entry.index as u64, locator, bytes);
            if let Some(format) = nested {
                member = member.with_nested_container_format(format);
            }
            member
        }
        Err(unavailable) => ContainerMember::unavailable(entry.index as u64, locator, unavailable),
    })
}

fn read_zip_member_bytes(
    archive: Option<&mut zip::ZipArchive<Cursor<&[u8]>>>,
    entry: &ZipEntryInfo,
) -> Result<Vec<u8>, ContainerUnavailable> {
    let Some(archive) = archive else {
        return Err(ContainerUnavailable::new(
            ContainerUnavailableKind::Unsupported,
            "grist.archive.zip_decoder_unavailable",
        )
        .with_message(
            "the ZIP decoder could not open this method combination; inventory is preserved",
        ));
    };
    match archive.by_index(entry.index) {
        Err(error) => Err(ContainerUnavailable::new(
            ContainerUnavailableKind::Unsupported,
            "grist.archive.unsupported_compression",
        )
        .with_message(format!(
            "cannot decode ZIP member {} with method {}: {error}",
            entry.path, entry.metadata.compression_method
        ))),
        Ok(file) => {
            let mut bounded = file.take(entry.metadata.uncompressed_size.saturating_add(1));
            let mut bytes = Vec::with_capacity(
                usize::try_from(entry.metadata.uncompressed_size)
                    .unwrap_or(usize::MAX)
                    .min(16 * 1024 * 1024),
            );
            bounded.read_to_end(&mut bytes).map_err(|error| {
                ContainerUnavailable::new(
                    ContainerUnavailableKind::Failed,
                    "grist.archive.zip_member_read_failed",
                )
                .with_message(format!("cannot read ZIP member {}: {error}", entry.path))
            })?;
            if bytes.len() as u64 != entry.metadata.uncompressed_size {
                return Err(ContainerUnavailable::new(
                    ContainerUnavailableKind::Failed,
                    "grist.archive.zip_member_size_mismatch",
                )
                .with_message(format!(
                    "ZIP member {} expanded length disagrees with the central directory",
                    entry.path
                )));
            }
            Ok(bytes)
        }
    }
}
fn decode_zip(
    context: &ContainerDecodeContext<'_>,
) -> Result<Vec<ContainerMember>, ContainerDecodeFailure> {
    let zip64 = is_zip64(context.bytes());
    let mut entries = zip_central_directory(context.bytes(), context)?;
    let mut compressed = 0_u64;
    let mut expanded = 0_u64;
    let mut collision_groups = BTreeMap::<String, Vec<usize>>::new();
    let mut archive = zip::ZipArchive::new(Cursor::new(context.bytes())).ok();

    for entry in &mut entries {
        context.checkpoint()?;
        compressed = compressed.saturating_add(entry.metadata.compressed_size);
        expanded = expanded.saturating_add(entry.metadata.uncompressed_size);
        entry.metadata.zip64 |= zip64;
    }

    context.check_archive_preflight(entries.len() as u64, compressed, expanded)?;
    for entry in &mut entries {
        context.checkpoint()?;
        if entry.metadata.entry_kind == ArchiveEntryKind::SymbolicLink
            && !entry.metadata.encrypted
            && zip_compression_supported(&entry.metadata.compression_method)
        {
            context.check_member_capacity(entry.metadata.uncompressed_size)?;
            match read_zip_member_bytes(archive.as_mut(), entry) {
                Ok(bytes) => {
                    entry.metadata.link_target =
                        Some(String::from_utf8_lossy(&bytes).replace('\u{5c}', "/"));
                }
                Err(unavailable) => entry.unavailable = Some(unavailable),
            }
        }
        let validation =
            context
                .archive_security_policy()
                .validate_member(&ArchiveMemberDescriptor {
                    path: &entry.path,
                    kind: entry.metadata.entry_kind,
                    link_target: entry.metadata.link_target.as_deref(),
                });
        entry.rejected = validation
            .as_ref()
            .err()
            .map(|error| (error.code, error.message.clone()));
        if let Ok(key) = validation {
            collision_groups.entry(key).or_default().push(entry.index);
        }
    }
    for indexes in collision_groups
        .values()
        .filter(|indexes| indexes.len() > 1)
    {
        for index in indexes {
            entries[*index].rejected = Some((
                "grist.security.archive.duplicate_path",
                "archive path is ambiguous after cross-platform normalization".into(),
            ));
        }
    }

    let mut members = Vec::with_capacity(entries.len());
    for entry in entries {
        context.checkpoint()?;
        let locator = member_locator(entry.index, &entry.path)?;
        let unavailable = if let Some((code, message)) = &entry.rejected {
            Some(
                ContainerUnavailable::new(ContainerUnavailableKind::Rejected, *code)
                    .with_message(message),
            )
        } else if let Some(unavailable) = entry.unavailable.clone() {
            Some(unavailable)
        } else if entry.metadata.encrypted {
            Some(
                ContainerUnavailable::new(
                    ContainerUnavailableKind::Encrypted,
                    "grist.archive.encrypted_member",
                )
                .with_message(
                    "encrypted ZIP members require an explicitly selected decryption provider",
                ),
            )
        } else if !zip_compression_supported(&entry.metadata.compression_method) {
            Some(
                ContainerUnavailable::new(
                    ContainerUnavailableKind::Unsupported,
                    "grist.archive.unsupported_compression",
                )
                .with_message(format!(
                    "ZIP compression method {} is inventoried but unsupported",
                    entry.metadata.compression_method
                )),
            )
        } else if !matches!(
            entry.metadata.entry_kind,
            ArchiveEntryKind::RegularFile | ArchiveEntryKind::Directory
        ) {
            Some(
                ContainerUnavailable::new(
                    ContainerUnavailableKind::Rejected,
                    archive_kind_code(entry.metadata.entry_kind),
                )
                .with_message("archive links and device entries are inventory-only"),
            )
        } else {
            None
        };

        let mut member = if let Some(unavailable) = unavailable {
            ContainerMember::unavailable(entry.index as u64, locator, unavailable)
        } else if entry.metadata.entry_kind == ArchiveEntryKind::Directory {
            ContainerMember::available(entry.index as u64, locator, Vec::new())
        } else {
            materialize_zip_member(context, archive.as_mut(), &entry, locator)?
        };
        member = member
            .with_declared_filename(entry.path.clone())
            .with_compressed_bytes(entry.metadata.compressed_size)
            .with_entry_kind(entry.metadata.entry_kind)
            .with_archive_metadata(entry.metadata.clone());
        if let Some(target) = &entry.metadata.link_target {
            member = member.with_link_target(target.clone());
        }
        if let Some(hint) = member_hint(&entry.path) {
            member = member.with_format_hint(hint);
        }
        members.push(member);
    }
    Ok(members)
}

#[derive(Clone)]
struct TarEntryInfo {
    index: usize,
    path: String,
    metadata: ArchiveMemberMetadata,
    supported_file: bool,
    rejected: Option<(&'static str, String)>,
}

fn decode_tar(
    context: &ContainerDecodeContext<'_>,
) -> Result<Vec<ContainerMember>, ContainerDecodeFailure> {
    if !is_tar(context.bytes()) {
        return Err(malformed("invalid TAR archive signature or checksum"));
    }
    let mut archive = tar::Archive::new(Cursor::new(context.bytes()));
    let iterator = archive
        .entries()
        .map_err(|error| malformed(format!("invalid TAR archive: {error}")))?;
    let mut entries = Vec::new();
    let mut collision_groups = BTreeMap::<String, Vec<usize>>::new();
    let mut expanded = 0_u64;

    for (index, result) in iterator.enumerate() {
        context.check_archive_member_count((index as u64).saturating_add(1))?;
        context.checkpoint()?;
        let entry = result
            .map_err(|error| malformed(format!("cannot inspect TAR member {index}: {error}")))?;
        let path = String::from_utf8_lossy(&entry.path_bytes()).replace('\u{5c}', "/");
        let header = entry.header();
        let tar_type = header.entry_type();
        let entry_kind = tar_entry_kind(tar_type);
        let link_target = entry
            .link_name_bytes()
            .map(|value| String::from_utf8_lossy(&value).replace('\u{5c}', "/"));
        let validation =
            context
                .archive_security_policy()
                .validate_member(&ArchiveMemberDescriptor {
                    path: &path,
                    kind: entry_kind,
                    link_target: link_target.as_deref(),
                });
        let rejected = validation
            .as_ref()
            .err()
            .map(|error| (error.code, error.message.clone()));
        if let Ok(key) = validation {
            collision_groups.entry(key).or_default().push(index);
        }
        let size = entry.size();
        expanded = expanded.saturating_add(size);
        entries.push(TarEntryInfo {
            index,
            path,
            metadata: ArchiveMemberMetadata {
                entry_kind,
                compression_method: "stored".into(),
                compressed_size: size,
                uncompressed_size: size,
                crc32: None,
                mode: header.mode().ok(),
                uid: header.uid().ok(),
                gid: header.gid().ok(),
                modified_time: header.mtime().ok(),
                link_target,
                encrypted: false,
                zip64: false,
            },
            supported_file: tar_type.is_file() || tar_type.is_contiguous() || tar_type.is_dir(),
            rejected,
        });
    }

    context.check_archive_preflight(
        entries.len() as u64,
        context.bytes().len() as u64,
        expanded,
    )?;
    for indexes in collision_groups
        .values()
        .filter(|indexes| indexes.len() > 1)
    {
        for index in indexes {
            entries[*index].rejected = Some((
                "grist.security.archive.duplicate_path",
                "archive path is ambiguous after cross-platform normalization".into(),
            ));
        }
    }

    let mut second = tar::Archive::new(Cursor::new(context.bytes()));
    let mut iterator = second
        .entries()
        .map_err(|error| malformed(format!("invalid TAR archive: {error}")))?;
    let mut members = Vec::with_capacity(entries.len());
    for entry in entries {
        context.checkpoint()?;
        let mut source = iterator
            .next()
            .ok_or_else(|| malformed("TAR entry sequence changed between inventory and read"))?
            .map_err(|error| {
                malformed(format!("cannot read TAR member {}: {error}", entry.path))
            })?;
        let locator = member_locator(entry.index, &entry.path)?;
        let unavailable = if let Some((code, message)) = &entry.rejected {
            Some(
                ContainerUnavailable::new(ContainerUnavailableKind::Rejected, *code)
                    .with_message(message),
            )
        } else if !entry.supported_file {
            Some(
                ContainerUnavailable::new(
                    ContainerUnavailableKind::Unsupported,
                    "grist.archive.tar_unsupported_entry",
                )
                .with_message("the TAR entry type is inventoried but not materialized"),
            )
        } else if !matches!(
            entry.metadata.entry_kind,
            ArchiveEntryKind::RegularFile | ArchiveEntryKind::Directory
        ) {
            Some(
                ContainerUnavailable::new(
                    ContainerUnavailableKind::Rejected,
                    archive_kind_code(entry.metadata.entry_kind),
                )
                .with_message("archive links and device entries are inventory-only"),
            )
        } else {
            None
        };
        let mut member = if let Some(unavailable) = unavailable {
            ContainerMember::unavailable(entry.index as u64, locator, unavailable)
        } else if entry.metadata.entry_kind == ArchiveEntryKind::Directory {
            ContainerMember::available(entry.index as u64, locator, Vec::new())
        } else {
            context.check_member_capacity(entry.metadata.uncompressed_size)?;
            let mut bytes = Vec::with_capacity(
                usize::try_from(entry.metadata.uncompressed_size)
                    .unwrap_or(usize::MAX)
                    .min(16 * 1024 * 1024),
            );
            (&mut source)
                .take(entry.metadata.uncompressed_size.saturating_add(1))
                .read_to_end(&mut bytes)
                .map_err(|error| {
                    malformed(format!("cannot read TAR member {}: {error}", entry.path))
                })?;
            if bytes.len() as u64 != entry.metadata.uncompressed_size {
                return Err(malformed(format!(
                    "TAR member {} expanded length disagrees with its header",
                    entry.path
                )));
            }
            let nested = nested_archive_format(&bytes);
            let mut member = ContainerMember::available(entry.index as u64, locator, bytes);
            if let Some(format) = nested {
                member = member.with_nested_container_format(format);
            }
            member
        };
        member = member
            .with_declared_filename(entry.path.clone())
            .with_compressed_bytes(entry.metadata.compressed_size)
            .with_entry_kind(entry.metadata.entry_kind)
            .with_archive_metadata(entry.metadata.clone());
        if let Some(target) = &entry.metadata.link_target {
            member = member.with_link_target(target.clone());
        }
        if let Some(hint) = member_hint(&entry.path) {
            member = member.with_format_hint(hint);
        }
        members.push(member);
    }
    Ok(members)
}

fn member_locator(
    index: usize,
    path: &str,
) -> Result<ParentRelativeLocator, ContainerDecodeFailure> {
    let locator_path = if path.is_empty() || path.chars().any(char::is_control) {
        format!("invalid-member-{index}")
    } else {
        path.to_string()
    };
    ParentRelativeLocator::single(LocationComponent::ArchiveMember {
        member_path: locator_path,
        member_index: IndexPosition::zero_based(index as u64),
    })
    .map_err(|error| malformed(format!("invalid archive member locator: {error}")))
}

fn zip_entry_kind(directory: bool, symlink: bool, mode: Option<u32>) -> ArchiveEntryKind {
    if directory {
        return ArchiveEntryKind::Directory;
    }
    if symlink {
        return ArchiveEntryKind::SymbolicLink;
    }
    match mode.map(|value| value & 0o170000) {
        Some(0o120000) => ArchiveEntryKind::SymbolicLink,
        Some(0o060000) => ArchiveEntryKind::BlockDevice,
        Some(0o020000) => ArchiveEntryKind::CharacterDevice,
        Some(0o010000) => ArchiveEntryKind::Fifo,
        Some(0o140000) => ArchiveEntryKind::Socket,
        Some(0o040000) => ArchiveEntryKind::Directory,
        _ => ArchiveEntryKind::RegularFile,
    }
}

fn tar_entry_kind(value: tar::EntryType) -> ArchiveEntryKind {
    if value.is_dir() {
        ArchiveEntryKind::Directory
    } else if value.is_symlink() {
        ArchiveEntryKind::SymbolicLink
    } else if value.is_hard_link() {
        ArchiveEntryKind::HardLink
    } else if value.is_block_special() {
        ArchiveEntryKind::BlockDevice
    } else if value.is_character_special() {
        ArchiveEntryKind::CharacterDevice
    } else if value.is_fifo() {
        ArchiveEntryKind::Fifo
    } else if value.is_file() || value.is_contiguous() {
        ArchiveEntryKind::RegularFile
    } else {
        ArchiveEntryKind::Socket
    }
}

fn archive_kind_code(kind: ArchiveEntryKind) -> &'static str {
    match kind {
        ArchiveEntryKind::SymbolicLink | ArchiveEntryKind::HardLink => {
            "grist.security.archive.unsafe_link"
        }
        ArchiveEntryKind::BlockDevice
        | ArchiveEntryKind::CharacterDevice
        | ArchiveEntryKind::Fifo
        | ArchiveEntryKind::Socket => "grist.security.archive.device_file",
        ArchiveEntryKind::RegularFile | ArchiveEntryKind::Directory => {
            "grist.archive.unsupported_entry"
        }
    }
}

fn nested_archive_format(bytes: &[u8]) -> Option<&'static str> {
    if is_zip(bytes) {
        Some("zip")
    } else if is_tar(bytes) {
        Some("tar")
    } else {
        None
    }
}

fn member_hint(path: &str) -> Option<FormatHint> {
    let extension = path.rsplit('.').next()?.to_ascii_lowercase();
    match extension.as_str() {
        "zip" => Some(FormatHint::exact("zip")),
        "tar" => Some(FormatHint::exact("tar")),
        "txt" => Some(FormatHint::exact("text")),
        "md" | "markdown" => Some(FormatHint::exact("markdown")),
        "json" => Some(FormatHint::exact("json")),
        "xml" => Some(FormatHint::exact("xml")),
        "pdf" => Some(FormatHint::exact("pdf")),
        _ => None,
    }
}

fn is_zip(bytes: &[u8]) -> bool {
    bytes.starts_with(b"PK\x03\x04")
        || bytes.starts_with(b"PK\x05\x06")
        || bytes.starts_with(b"PK\x07\x08")
}

fn is_zip64(bytes: &[u8]) -> bool {
    bytes.windows(4).any(|window| window == b"PK\x06\x06")
        || bytes.windows(4).any(|window| window == b"PK\x06\x07")
        || central_directory_uses_zip64(bytes)
}

fn central_directory_uses_zip64(bytes: &[u8]) -> bool {
    let mut offset = 0_usize;
    while offset.saturating_add(46) <= bytes.len() {
        if bytes.get(offset..offset + 4) != Some(b"PK\x01\x02".as_slice()) {
            offset += 1;
            continue;
        }
        let Some(name_len) = little_u16(bytes, offset + 28) else {
            return false;
        };
        let Some(extra_len) = little_u16(bytes, offset + 30) else {
            return false;
        };
        let Some(comment_len) = little_u16(bytes, offset + 32) else {
            return false;
        };
        let extra_start = offset
            .saturating_add(46)
            .saturating_add(usize::from(name_len));
        let extra_end = extra_start.saturating_add(usize::from(extra_len));
        let Some(extra) = bytes.get(extra_start..extra_end) else {
            return false;
        };
        let mut cursor = 0_usize;
        while cursor.saturating_add(4) <= extra.len() {
            let id = u16::from_le_bytes([extra[cursor], extra[cursor + 1]]);
            let size = usize::from(u16::from_le_bytes([extra[cursor + 2], extra[cursor + 3]]));
            if id == 0x0001 {
                return true;
            }
            cursor = cursor.saturating_add(4).saturating_add(size);
        }
        offset = extra_end.saturating_add(usize::from(comment_len));
    }
    false
}

fn little_u16(bytes: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_le_bytes([
        *bytes.get(offset)?,
        *bytes.get(offset.checked_add(1)?)?,
    ]))
}

fn little_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes([
        *bytes.get(offset)?,
        *bytes.get(offset.checked_add(1)?)?,
        *bytes.get(offset.checked_add(2)?)?,
        *bytes.get(offset.checked_add(3)?)?,
    ]))
}

fn little_u64(bytes: &[u8], offset: usize) -> Option<u64> {
    Some(u64::from_le_bytes([
        *bytes.get(offset)?,
        *bytes.get(offset.checked_add(1)?)?,
        *bytes.get(offset.checked_add(2)?)?,
        *bytes.get(offset.checked_add(3)?)?,
        *bytes.get(offset.checked_add(4)?)?,
        *bytes.get(offset.checked_add(5)?)?,
        *bytes.get(offset.checked_add(6)?)?,
        *bytes.get(offset.checked_add(7)?)?,
    ]))
}

pub fn is_tar(bytes: &[u8]) -> bool {
    if bytes.len() < 512 {
        return false;
    }
    if bytes[..512].iter().all(|byte| *byte == 0) {
        return bytes.len() >= 1024 && bytes[512..1024].iter().all(|byte| *byte == 0);
    }
    let stored = parse_tar_octal(&bytes[148..156]);
    let Some(stored) = stored else {
        return false;
    };
    let calculated = bytes[..512]
        .iter()
        .enumerate()
        .map(|(index, byte)| {
            if (148..156).contains(&index) {
                u64::from(b' ')
            } else {
                u64::from(*byte)
            }
        })
        .sum::<u64>();
    stored == calculated
}

fn parse_tar_octal(bytes: &[u8]) -> Option<u64> {
    let text = std::str::from_utf8(bytes).ok()?.trim_matches(['\0', ' ']);
    (!text.is_empty())
        .then(|| u64::from_str_radix(text, 8).ok())
        .flatten()
}

fn normalize_format(value: &str) -> String {
    value
        .trim()
        .to_ascii_lowercase()
        .replace(['-', ' '], "_")
        .replace("zip64", "zip")
}

fn malformed(message: impl Into<String>) -> ContainerDecodeFailure {
    ContainerDecodeFailure::malformed(PARSER, message)
}

impl ToDocumentGraph for ArchiveDocument {
    fn to_document_graph(
        &self,
        context: DocumentGraphContext,
    ) -> Result<DocumentGraph, TransformError> {
        let ids = context
            .identity_generator(SchemaVersion::ARCHIVE_V1, "archive")
            .map_err(graph_error)?;
        let mut graph = DocumentGraph::new(context.graph_id, DocumentKind::Other("archive".into()))
            .with_projection(
                "archive",
                SchemaVersion::ARCHIVE_V1,
                "grist.archive.to-document-graph.v1",
            );
        graph.source = context.source;
        graph.dialect = context.dialect;
        graph.attrs = context.attrs;
        graph.attrs.insert(
            "format".into(),
            serde_json::to_value(self.format).map_err(graph_error)?,
        );
        let root = graph_node_id(&ids, &["archive"], "archive", None)?;
        graph.add_node(DocumentNode::new(&root, DocumentNodeKind::Container).with_ordinal(0));
        add_graph_children(
            &mut graph,
            &ids,
            &root,
            &self.traversal.children,
            &["archive"],
        )?;
        graph.finalize_projection(&ids).map_err(graph_error)?;
        Ok(graph)
    }
}

fn add_graph_children(
    graph: &mut DocumentGraph,
    ids: &GraphIdGenerator,
    parent: &str,
    children: &[crate::container::ContainerChild],
    parent_path: &[&str],
) -> Result<(), TransformError> {
    for child in children {
        let name = child
            .artifact
            .declared_filename
            .as_deref()
            .unwrap_or("archive-member");
        let order = child.source_order.to_string();
        let mut path = parent_path.to_vec();
        path.push(&order);
        let id = graph_node_id(
            ids,
            &path,
            &format!("{}:{name}", child.source_order),
            Some(child.artifact.locator.clone()),
        )?;
        let mut node = DocumentNode::new(&id, DocumentNodeKind::ArchiveMember)
            .with_name(name)
            .with_locator(child.artifact.locator.clone())
            .with_ordinal(child.source_order as usize)
            .with_attr("status", format!("{:?}", child.status).to_ascii_lowercase())
            .with_attr(
                "artifact_identity",
                serde_json::to_value(&child.artifact.identity).map_err(graph_error)?,
            );
        if let Some(metadata) = &child.archive_metadata {
            node = node.with_attr(
                "archive_metadata",
                serde_json::to_value(metadata).map_err(graph_error)?,
            );
        }
        graph.add_node(node);
        graph.add_contains(parent, &id);
        add_graph_children(graph, ids, &id, &child.children, &path)?;
    }
    Ok(())
}

fn graph_node_id(
    ids: &GraphIdGenerator,
    path: &[&str],
    native: &str,
    locator: Option<crate::core::SourceLocator>,
) -> Result<String, TransformError> {
    let mut address = ProjectionAddress::native(path.iter().copied(), native);
    if let Some(locator) = locator {
        address = address.with_locator(locator);
    }
    ids.node_id(&address).map_err(graph_error)
}

fn graph_error(value: impl std::fmt::Display) -> TransformError {
    TransformError::Other {
        message: value.to_string(),
    }
}
