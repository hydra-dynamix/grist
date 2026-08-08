use super::cfb::{self, CfbEntry, CfbEntryKind, CfbFile};
use super::mapi::{
    PropertySet, binary, decode_bytes, entry_locator, parse_named_properties, parse_property_set,
    property, property_binary, property_date, property_i32, property_text,
};
use super::*;
use crate::container::{
    ArtifactDisposition, ArtifactMetadata, ArtifactParent, ArtifactRelationship, EmbeddedArtifact,
};
use crate::core::{
    ContentIdentity, Diagnostic, FormatIdentity, IndexBase, IndexPosition, LocationComponent,
    OperationControl, SourceInfo, SourceLocator,
};
use std::collections::BTreeSet;

const PARSER: &str = "grist.outlook.msg";

pub(super) fn parse_document(
    bytes: &[u8],
    _source: &SourceInfo,
    options: &OutlookMsgOptions,
    control: &OperationControl,
) -> Result<OutlookMsgDocument, Box<Diagnostic>> {
    validate_options(options)?;
    let cfb = cfb::parse(bytes, options.max_chain_sectors)?;
    let parent_identity = ContentIdentity::for_raw_bytes(bytes).with_format(FormatIdentity::new(
        "msg",
        Some("application/vnd.ms-outlook"),
    ));
    let mut diagnostics = cfb.diagnostics.clone();
    let named = parse_named_properties(&cfb, &mut diagnostics);
    let mut state = ParseState {
        cfb: &cfb,
        options,
        control,
        parent_identity,
        named,
        diagnostics,
    };
    let mut document = state.parse_message_object("", 0, 1252);
    document.diagnostics = state.diagnostics;
    document.complete = document
        .diagnostics
        .iter()
        .all(|diagnostic| !diagnostic.partial)
        && !document.encrypted;
    Ok(document)
}

fn validate_options(options: &OutlookMsgOptions) -> Result<(), Box<Diagnostic>> {
    if options.max_properties_per_object == 0
        || options.max_recipients == 0
        || options.max_attachments == 0
        || options.max_embedded_depth == 0
        || options.max_chain_sectors == 0
        || options.max_rtf_output_bytes == 0
    {
        return Err(Box::new(Diagnostic::malformed(
            PARSER,
            "all MSG structural and output limits must be greater than zero",
        )));
    }
    Ok(())
}

struct ParseState<'a> {
    cfb: &'a CfbFile,
    options: &'a OutlookMsgOptions,
    control: &'a OperationControl,
    parent_identity: ContentIdentity,
    named: Vec<MapiNamedProperty>,
    diagnostics: Vec<Diagnostic>,
}

impl ParseState<'_> {
    fn parse_message_object(
        &mut self,
        storage_path: &str,
        depth: usize,
        inherited_codepage: u32,
    ) -> OutlookMsgDocument {
        if let Err(error) = self.control.budget().observe_nesting_depth(depth as u64) {
            self.diagnostics.push(error.diagnostic(PARSER).partial());
        }
        let before = self.diagnostics.len();
        let PropertySet {
            properties,
            mut consumed_paths,
            codepage,
        } = parse_property_set(
            self.cfb,
            storage_path,
            &self.named,
            inherited_codepage,
            self.options,
            &mut self.diagnostics,
        );
        let _ = self
            .control
            .budget()
            .consume_nodes(properties.len() as u64)
            .map_err(|error| {
                self.diagnostics.push(error.diagnostic(PARSER).partial());
            });
        let recipients = self.parse_recipients(storage_path, codepage, &mut consumed_paths);
        let attachments =
            self.parse_attachments(storage_path, depth, codepage, &mut consumed_paths);
        let bodies = self.parse_bodies(&properties, codepage);
        let subject =
            property_text(&properties, 0x0037).or_else(|| property_text(&properties, 0x0e1d));
        let message_class = property_text(&properties, 0x001a);
        let class = message_class
            .as_ref()
            .map(|value| value.text.to_ascii_uppercase())
            .unwrap_or_default();
        let signed = class.contains("SMIME.MULTIPARTSIGNED");
        let encrypted = class.contains("SMIME") && !signed;
        if encrypted {
            self.diagnostics.push(
                Diagnostic::warning(
                    PARSER,
                    "outlook.msg.encrypted",
                    "S/MIME encrypted MSG content requires an explicit decryption provider",
                )
                .partial(),
            );
        }
        let sender = MsgSender {
            display_name: property_text(&properties, 0x0c1a)
                .or_else(|| property_text(&properties, 0x0042)),
            email_address: property_text(&properties, 0x0c1f)
                .or_else(|| property_text(&properties, 0x0065)),
            smtp_address: property_text(&properties, 0x5d01),
            address_type: property_text(&properties, 0x0c1e)
                .or_else(|| property_text(&properties, 0x0064)),
        };
        let thread = MsgThreadEvidence {
            internet_message_id: property_text(&properties, 0x1035),
            in_reply_to: property_text(&properties, 0x1042)
                .map(|fact| message_ids(&fact.text))
                .unwrap_or_default(),
            references: property_text(&properties, 0x1039)
                .map(|fact| message_ids(&fact.text))
                .unwrap_or_default(),
            conversation_topic: property_text(&properties, 0x0070),
            conversation_index: property_binary(&properties, 0x0071),
        };
        let dates = date_facts(&properties);
        let unknown_objects = self.unknown_direct_children(storage_path, &consumed_paths);
        let local_complete = self.diagnostics[before..]
            .iter()
            .all(|diagnostic| !diagnostic.partial)
            && !encrypted;
        OutlookMsgDocument {
            schema_version: crate::core::SchemaVersion::OUTLOOK_MSG_V1.to_string(),
            compound_file: self.cfb.metadata.clone(),
            properties,
            named_properties: self.named.clone(),
            recipients,
            bodies,
            attachments,
            subject,
            sender,
            thread,
            dates,
            message_class,
            unknown_objects,
            encrypted,
            signed,
            diagnostics: self.diagnostics[before..].to_vec(),
            complete: local_complete,
        }
    }

    fn parse_recipients(
        &mut self,
        storage_path: &str,
        codepage: u32,
        parent_consumed: &mut BTreeSet<String>,
    ) -> Vec<MsgRecipient> {
        let mut paths = self
            .cfb
            .direct_children(storage_path)
            .filter(|entry| {
                entry.kind == CfbEntryKind::Storage
                    && entry
                        .name
                        .to_ascii_lowercase()
                        .starts_with("__recip_version1.0_#")
            })
            .map(|entry| entry.path.clone())
            .collect::<Vec<_>>();
        paths.sort();
        if paths.len() > self.options.max_recipients {
            self.diagnostics.push(partial(
                "outlook.msg.limit.recipients",
                "recipient count exceeds max_recipients",
            ));
            paths.truncate(self.options.max_recipients);
        }
        let mut recipients = Vec::new();
        for (ordinal, path) in paths.into_iter().enumerate() {
            parent_consumed.insert(path.clone());
            let set = parse_property_set(
                self.cfb,
                &path,
                &self.named,
                codepage,
                self.options,
                &mut self.diagnostics,
            );
            if let Err(error) = self
                .control
                .budget()
                .consume_nodes(1 + set.properties.len() as u64)
            {
                self.diagnostics.push(error.diagnostic(PARSER).partial());
            }
            let recipient_type = match property_i32(&set.properties, 0x0c15) {
                Some(1) => MsgRecipientType::To,
                Some(2) => MsgRecipientType::Cc,
                Some(3) => MsgRecipientType::Bcc,
                Some(0) => MsgRecipientType::Originator,
                _ => MsgRecipientType::Unknown,
            };
            let locator = self
                .cfb
                .entry(&path)
                .map(storage_locator)
                .expect("recipient storage path exists");
            let unknown_objects = self.unknown_direct_children(&path, &set.consumed_paths);
            recipients.push(MsgRecipient {
                ordinal,
                storage_path: path,
                recipient_type,
                display_name: property_text(&set.properties, 0x3001),
                email_address: property_text(&set.properties, 0x3003),
                smtp_address: property_text(&set.properties, 0x39fe),
                address_type: property_text(&set.properties, 0x3002),
                properties: set.properties,
                unknown_objects,
                locator,
            });
        }
        recipients
    }

    fn parse_attachments(
        &mut self,
        storage_path: &str,
        depth: usize,
        codepage: u32,
        parent_consumed: &mut BTreeSet<String>,
    ) -> Vec<MsgAttachment> {
        let mut paths = self
            .cfb
            .direct_children(storage_path)
            .filter(|entry| {
                entry.kind == CfbEntryKind::Storage
                    && entry
                        .name
                        .to_ascii_lowercase()
                        .starts_with("__attach_version1.0_#")
            })
            .map(|entry| entry.path.clone())
            .collect::<Vec<_>>();
        paths.sort();
        if paths.len() > self.options.max_attachments {
            self.diagnostics.push(partial(
                "outlook.msg.limit.attachments",
                "attachment count exceeds max_attachments",
            ));
            paths.truncate(self.options.max_attachments);
        }
        paths
            .into_iter()
            .enumerate()
            .map(|(ordinal, path)| {
                parent_consumed.insert(path.clone());
                self.parse_attachment(path, ordinal, depth, codepage)
            })
            .collect()
    }

    fn parse_attachment(
        &mut self,
        path: String,
        ordinal: usize,
        depth: usize,
        codepage: u32,
    ) -> MsgAttachment {
        let mut set = parse_property_set(
            self.cfb,
            &path,
            &self.named,
            codepage,
            self.options,
            &mut self.diagnostics,
        );
        let method_code = property_i32(&set.properties, 0x3705).unwrap_or_default();
        if let Err(error) = self
            .control
            .budget()
            .consume_nodes(1 + set.properties.len() as u64)
        {
            self.diagnostics.push(error.diagnostic(PARSER).partial());
        }
        let method = attachment_method(method_code);
        let filename = property_text(&set.properties, 0x3707)
            .or_else(|| property_text(&set.properties, 0x3704));
        let mime_type = property_text(&set.properties, 0x370e);
        let content_id = property_text(&set.properties, 0x3712);
        let content_location = property_text(&set.properties, 0x3713);
        let data_path = format!("{path}/__substg1.0_37010102");
        let embedded_path = format!("{path}/__substg1.0_3701000D");
        let data = self.cfb.entry(&data_path);
        let embedded_storage = self.cfb.entry(&embedded_path);
        if data.is_some() {
            set.consumed_paths.insert(data_path.clone());
        }
        if embedded_storage.is_some() {
            set.consumed_paths.insert(embedded_path.clone());
        }
        let child_budget = self.control.budget().consume_child_artifacts(1);
        if let Err(error) = &child_budget {
            self.diagnostics.push(error.diagnostic(PARSER).partial());
        }
        let artifact = data.and_then(|entry| {
            if child_budget.is_err() {
                return None;
            }
            self.capture_attachment_artifact(
                entry,
                filename.as_ref().map(|fact| fact.text.as_str()),
                mime_type.as_ref().map(|fact| fact.text.as_str()),
                content_id.is_some(),
            )
        });
        let embedded_message = if embedded_storage.is_some()
            && child_budget.is_ok()
            && self.options.parse_embedded_messages
        {
            if depth + 1 > self.options.max_embedded_depth {
                self.diagnostics.push(partial(
                    "outlook.msg.limit.embedded_depth",
                    "embedded MSG exceeds max_embedded_depth",
                ));
                None
            } else {
                Some(Box::new(self.parse_message_object(
                    &embedded_path,
                    depth + 1,
                    codepage,
                )))
            }
        } else {
            None
        };
        let locator = self
            .cfb
            .entry(&path)
            .map(storage_locator)
            .expect("attachment storage path exists");
        let unknown_objects = self.unknown_direct_children(&path, &set.consumed_paths);
        MsgAttachment {
            ordinal,
            storage_path: path,
            method,
            filename,
            mime_type,
            content_id,
            content_location,
            rendering_position: property_i32(&set.properties, 0x370b),
            declared_size: property_i32(&set.properties, 0x0e20),
            properties: set.properties,
            artifact,
            embedded_message,
            unknown_objects,
            locator,
        }
    }

    fn capture_attachment_artifact(
        &mut self,
        entry: &CfbEntry,
        filename: Option<&str>,
        media_type: Option<&str>,
        inline_resource: bool,
    ) -> Option<EmbeddedArtifact> {
        let relationship = if inline_resource {
            ArtifactRelationship::InlineResourceOf
        } else {
            ArtifactRelationship::AttachmentOf
        };
        let disposition = if inline_resource {
            ArtifactDisposition::Inline
        } else {
            ArtifactDisposition::Attachment
        };
        let mut metadata = ArtifactMetadata::new(
            ArtifactParent::new(self.parent_identity.clone(), relationship),
            entry_locator(entry),
            disposition,
        );
        if let Some(filename) = filename {
            metadata = metadata.with_declared_filename(filename.to_string());
        }
        if let Some(media_type) = media_type {
            metadata = metadata.with_media_type(media_type.to_string());
        }
        let result = if self.options.inline_attachment_bytes {
            EmbeddedArtifact::capture_inline(metadata, &entry.data)
        } else {
            EmbeddedArtifact::inventory(metadata, &entry.data)
        };
        match result {
            Ok(artifact) => Some(artifact),
            Err(error) => {
                self.diagnostics.push(
                    Diagnostic::parser_defect(
                        PARSER,
                        format!("embedded-artifact invariant failed: {error}"),
                    )
                    .partial(),
                );
                None
            }
        }
    }

    fn parse_bodies(
        &mut self,
        properties: &[MapiProperty],
        codepage: u32,
    ) -> Vec<MsgBodyAlternative> {
        let mut bodies = Vec::new();
        if let Some(property) = property(properties, 0x1000)
            && let MapiValue::String {
                text,
                encoding,
                lossy,
                raw,
            } = &property.value
        {
            bodies.push(MsgBodyAlternative {
                ordinal: bodies.len(),
                kind: MsgBodyKind::PlainText,
                text: text.clone(),
                charset: encoding.clone(),
                lossy: *lossy,
                source_property_tag: property.property_tag.clone(),
                source_binary: raw.clone(),
                rtf_compression: None,
                active_content_inert: true,
                locator: property.locator.clone(),
            });
        }
        if let Some(property) = property(properties, 0x1013) {
            match &property.value {
                MapiValue::String {
                    text,
                    encoding,
                    lossy,
                    raw,
                } => {
                    bodies.push(MsgBodyAlternative {
                        ordinal: bodies.len(),
                        kind: MsgBodyKind::Html,
                        text: text.clone(),
                        charset: encoding.clone(),
                        lossy: *lossy,
                        source_property_tag: property.property_tag.clone(),
                        source_binary: raw.clone(),
                        rtf_compression: None,
                        active_content_inert: true,
                        locator: property.locator.clone(),
                    });
                }
                _ => {
                    if let Some(raw) = self.raw_property_bytes(property) {
                        let (text, charset, lossy) = decode_bytes(raw, codepage);
                        bodies.push(MsgBodyAlternative {
                            ordinal: bodies.len(),
                            kind: MsgBodyKind::Html,
                            text,
                            charset,
                            lossy,
                            source_property_tag: property.property_tag.clone(),
                            source_binary: binary(raw, self.options.inline_property_binary),
                            rtf_compression: None,
                            active_content_inert: true,
                            locator: property.locator.clone(),
                        });
                    }
                }
            }
        }
        if let Some(property) = property(properties, 0x1009)
            && let Some(raw) = self.raw_property_bytes(property).map(<[u8]>::to_vec)
        {
            match super::rtf::decompress(&raw, self.options.max_rtf_output_bytes) {
                Ok((rtf, compression)) => {
                    if !compression.crc_matches {
                        self.diagnostics.push(partial(
                            "outlook.msg.rtf.crc_mismatch",
                            "compressed RTF checksum does not match its header",
                        ));
                    }
                    let text = String::from_utf8_lossy(&rtf);
                    let lossy = matches!(text, std::borrow::Cow::Owned(_));
                    bodies.push(MsgBodyAlternative {
                        ordinal: bodies.len(),
                        kind: MsgBodyKind::Rtf,
                        text: text.into_owned(),
                        charset: "rtf-byte-stream".to_string(),
                        lossy,
                        source_property_tag: property.property_tag.clone(),
                        source_binary: binary(&raw, self.options.inline_property_binary),
                        rtf_compression: Some(compression),
                        active_content_inert: true,
                        locator: property.locator.clone(),
                    });
                }
                Err(message) => self
                    .diagnostics
                    .push(partial("outlook.msg.rtf.decompression_failed", message)),
            }
        }
        bodies
    }

    fn raw_property_bytes<'a>(&'a self, property: &'a MapiProperty) -> Option<&'a [u8]> {
        property
            .stream_paths
            .first()
            .and_then(|path| self.cfb.entry(path))
            .map(|entry| entry.data.as_slice())
    }

    fn unknown_direct_children(
        &self,
        storage_path: &str,
        consumed: &BTreeSet<String>,
    ) -> Vec<MsgUnknownObject> {
        self.cfb
            .direct_children(storage_path)
            .filter(|entry| {
                !(consumed.contains(&entry.path)
                    || storage_path.is_empty()
                        && entry.name.eq_ignore_ascii_case("__nameid_version1.0"))
            })
            .map(|entry| MsgUnknownObject {
                path: entry.path.clone(),
                kind: match entry.kind {
                    CfbEntryKind::Stream => MsgUnknownObjectKind::Stream,
                    CfbEntryKind::Storage | CfbEntryKind::Root => MsgUnknownObjectKind::Storage,
                },
                clsid: entry.clsid.clone(),
                data: (entry.kind == CfbEntryKind::Stream)
                    .then(|| binary(&entry.data, self.options.inline_unknown_streams)),
                locator: if entry.kind == CfbEntryKind::Stream {
                    entry_locator(entry)
                } else {
                    storage_locator(entry)
                },
            })
            .collect()
    }
}

fn attachment_method(value: i32) -> MsgAttachmentMethod {
    match value {
        0 => MsgAttachmentMethod::None,
        1 => MsgAttachmentMethod::ByValue,
        2 => MsgAttachmentMethod::ByReference,
        3 => MsgAttachmentMethod::ByReferenceResolve,
        4 => MsgAttachmentMethod::ByReferenceOnly,
        5 => MsgAttachmentMethod::EmbeddedMessage,
        6 => MsgAttachmentMethod::OleObject,
        _ => MsgAttachmentMethod::Unknown,
    }
}

fn date_facts(properties: &[MapiProperty]) -> Vec<MsgDateFact> {
    [
        (0x0039, "client_submit"),
        (0x0e06, "message_delivery"),
        (0x3007, "creation"),
        (0x3008, "last_modification"),
    ]
    .into_iter()
    .filter_map(|(id, role)| {
        let property = property(properties, id)?;
        Some(MsgDateFact {
            role: role.to_string(),
            property_tag: property.property_tag.clone(),
            value: property_date(properties, id)?,
            locator: property.locator.clone(),
        })
    })
    .collect()
}

fn message_ids(value: &str) -> Vec<String> {
    let mut ids = Vec::new();
    let mut rest = value;
    while let Some(start) = rest.find('<') {
        let candidate = &rest[start..];
        let Some(end) = candidate.find('>') else {
            break;
        };
        ids.push(candidate[..=end].to_string());
        rest = &candidate[end + 1..];
    }
    if ids.is_empty() {
        ids.extend(value.split_whitespace().map(str::to_string));
    }
    ids
}

fn storage_locator(entry: &CfbEntry) -> SourceLocator {
    SourceLocator::exact(LocationComponent::ArchiveMember {
        member_path: entry.path.clone(),
        member_index: IndexPosition::new(entry.id as u64, IndexBase::Zero)
            .expect("CFB directory ID is a valid zero-based index"),
    })
    .expect("CFB storage path is non-empty")
}

fn partial(code: impl Into<String>, message: impl Into<String>) -> Diagnostic {
    Diagnostic::warning(PARSER, code, message).partial()
}
