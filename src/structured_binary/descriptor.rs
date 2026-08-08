use super::model::ProtobufSyntax;
use super::wire::{Cursor, DecodeError};
use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub(crate) struct DescriptorPool {
    pub files: Vec<String>,
    pub messages: BTreeMap<String, MessageDescriptor>,
    pub enums: BTreeMap<String, EnumDescriptor>,
}

#[derive(Debug, Clone)]
pub(crate) struct MessageDescriptor {
    pub full_name: String,
    pub syntax: ProtobufSyntax,
    pub fields: BTreeMap<u32, FieldDescriptor>,
    pub map_entry: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct FieldDescriptor {
    pub name: String,
    pub json_name: String,
    pub number: u32,
    pub label: FieldLabel,
    pub kind: FieldKind,
    pub type_name: Option<String>,
    pub packed: bool,
    pub extension: bool,
    pub oneof: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FieldLabel {
    Optional,
    Required,
    Repeated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FieldKind {
    Double,
    Float,
    Int64,
    Uint64,
    Int32,
    Fixed64,
    Fixed32,
    Bool,
    String,
    Group,
    Message,
    Bytes,
    Uint32,
    Enum,
    Sfixed32,
    Sfixed64,
    Sint32,
    Sint64,
}

impl FieldKind {
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Double => "double",
            Self::Float => "float",
            Self::Int64 => "int64",
            Self::Uint64 => "uint64",
            Self::Int32 => "int32",
            Self::Fixed64 => "fixed64",
            Self::Fixed32 => "fixed32",
            Self::Bool => "bool",
            Self::String => "string",
            Self::Group => "group",
            Self::Message => "message",
            Self::Bytes => "bytes",
            Self::Uint32 => "uint32",
            Self::Enum => "enum",
            Self::Sfixed32 => "sfixed32",
            Self::Sfixed64 => "sfixed64",
            Self::Sint32 => "sint32",
            Self::Sint64 => "sint64",
        }
    }

    pub(crate) const fn wire_type(self) -> u8 {
        match self {
            Self::Double | Self::Fixed64 | Self::Sfixed64 => 1,
            Self::Float | Self::Fixed32 | Self::Sfixed32 => 5,
            Self::String | Self::Message | Self::Bytes => 2,
            Self::Group => 3,
            Self::Int64
            | Self::Uint64
            | Self::Int32
            | Self::Bool
            | Self::Uint32
            | Self::Enum
            | Self::Sint32
            | Self::Sint64 => 0,
        }
    }

    pub(crate) const fn packable(self) -> bool {
        !matches!(
            self,
            Self::String | Self::Message | Self::Bytes | Self::Group
        )
    }
}

#[derive(Debug, Clone)]
pub(crate) struct EnumDescriptor {
    pub values: BTreeMap<i32, String>,
}

#[derive(Debug, Default)]
struct FileProto {
    name: String,
    package: String,
    syntax: ProtobufSyntax,
    messages: Vec<MessageProto>,
    enums: Vec<EnumProto>,
    extensions: Vec<FieldProto>,
}

#[derive(Debug, Default)]
struct MessageProto {
    name: String,
    fields: Vec<FieldProto>,
    nested: Vec<MessageProto>,
    enums: Vec<EnumProto>,
    extensions: Vec<FieldProto>,
    oneofs: Vec<String>,
    map_entry: bool,
}

#[derive(Debug, Default)]
struct FieldProto {
    name: String,
    json_name: String,
    number: i32,
    label: i32,
    kind: i32,
    type_name: Option<String>,
    extendee: Option<String>,
    oneof_index: Option<usize>,
    packed: Option<bool>,
}

#[derive(Debug, Default)]
struct EnumProto {
    name: String,
    values: Vec<(String, i32)>,
}

pub(crate) fn parse_descriptor_set(bytes: &[u8]) -> Result<DescriptorPool, DecodeError> {
    if bytes.is_empty() {
        return Err(DecodeError::new(
            "protobuf.descriptor_empty",
            "FileDescriptorSet is empty",
            0,
        ));
    }
    let mut cursor = Cursor::new(bytes);
    let mut files = Vec::new();
    while !cursor.is_empty() {
        let (number, wire, start) = key(&mut cursor, "protobuf.descriptor_malformed")?;
        if number == 1 && wire == 2 {
            let payload = length_delimited(&mut cursor, "protobuf.descriptor_malformed")?;
            files.push(parse_file(payload)?);
        } else {
            skip_field(&mut cursor, wire, number, "protobuf.descriptor_malformed")?;
        }
        if cursor.position() <= start {
            return Err(DecodeError::new(
                "protobuf.descriptor_malformed",
                "descriptor parser did not advance",
                start,
            ));
        }
    }
    if files.is_empty() {
        return Err(DecodeError::new(
            "protobuf.descriptor_missing_files",
            "FileDescriptorSet contains no file descriptors",
            0,
        ));
    }
    build_pool(files)
}

fn build_pool(files: Vec<FileProto>) -> Result<DescriptorPool, DecodeError> {
    let file_names = files.iter().map(|file| file.name.clone()).collect();
    let mut pool = DescriptorPool {
        files: file_names,
        messages: BTreeMap::new(),
        enums: BTreeMap::new(),
    };
    let mut extensions = Vec::new();
    for file in files {
        let package = file.package.trim_matches('.');
        for descriptor in file.enums {
            add_enum(&mut pool, package, descriptor)?;
        }
        for descriptor in file.messages {
            add_message(&mut pool, package, file.syntax, descriptor, &mut extensions)?;
        }
        for extension in file.extensions {
            extensions.push((package.to_string(), file.syntax, extension));
        }
    }
    for (scope, syntax, field) in extensions {
        let Some(extendee) = field.extendee.clone() else {
            return Err(DecodeError::new(
                "protobuf.descriptor_invalid_extension",
                format!("extension {} has no extendee", field.name),
                0,
            ));
        };
        let target = normalize_type_name(&extendee, &scope);
        let oneofs = Vec::new();
        let descriptor = convert_field(field, syntax, &scope, &oneofs, true)?;
        let message = pool.messages.get_mut(&target).ok_or_else(|| {
            DecodeError::new(
                "protobuf.descriptor_unknown_extendee",
                format!("extension target {target} is not in the descriptor set"),
                0,
            )
        })?;
        if message
            .fields
            .insert(descriptor.number, descriptor)
            .is_some()
        {
            return Err(DecodeError::new(
                "protobuf.descriptor_duplicate_field",
                format!("message {target} has duplicate extension number"),
                0,
            ));
        }
    }
    Ok(pool)
}

fn add_message(
    pool: &mut DescriptorPool,
    scope: &str,
    syntax: ProtobufSyntax,
    descriptor: MessageProto,
    extensions: &mut Vec<(String, ProtobufSyntax, FieldProto)>,
) -> Result<(), DecodeError> {
    let full_name = join_name(scope, &descriptor.name);
    let mut fields = BTreeMap::new();
    for field in descriptor.fields {
        let field = convert_field(field, syntax, &full_name, &descriptor.oneofs, false)?;
        if fields.insert(field.number, field).is_some() {
            return Err(DecodeError::new(
                "protobuf.descriptor_duplicate_field",
                format!("message {full_name} has duplicate field number"),
                0,
            ));
        }
    }
    for field in descriptor.extensions {
        extensions.push((full_name.clone(), syntax, field));
    }
    let nested = descriptor.nested;
    let enums = descriptor.enums;
    let message = MessageDescriptor {
        full_name: full_name.clone(),
        syntax,
        fields,
        map_entry: descriptor.map_entry,
    };
    if pool.messages.insert(full_name.clone(), message).is_some() {
        return Err(DecodeError::new(
            "protobuf.descriptor_duplicate_message",
            format!("duplicate message descriptor {full_name}"),
            0,
        ));
    }
    for child in nested {
        add_message(pool, &full_name, syntax, child, extensions)?;
    }
    for child in enums {
        add_enum(pool, &full_name, child)?;
    }
    Ok(())
}

fn add_enum(
    pool: &mut DescriptorPool,
    scope: &str,
    descriptor: EnumProto,
) -> Result<(), DecodeError> {
    let full_name = join_name(scope, &descriptor.name);
    let mut values = BTreeMap::new();
    for (name, number) in descriptor.values {
        values.entry(number).or_insert(name);
    }
    if pool
        .enums
        .insert(full_name.clone(), EnumDescriptor { values })
        .is_some()
    {
        return Err(DecodeError::new(
            "protobuf.descriptor_duplicate_enum",
            format!("duplicate enum descriptor {full_name}"),
            0,
        ));
    }
    Ok(())
}

fn parse_file(bytes: &[u8]) -> Result<FileProto, DecodeError> {
    let mut cursor = Cursor::new(bytes);
    let mut file = FileProto {
        syntax: ProtobufSyntax::Proto2,
        ..Default::default()
    };
    let mut edition_present = false;
    while !cursor.is_empty() {
        let (number, wire, _) = key(&mut cursor, "protobuf.descriptor_malformed")?;
        match (number, wire) {
            (1, 2) => file.name = string(&mut cursor, "protobuf.descriptor_malformed")?,
            (2, 2) => file.package = string(&mut cursor, "protobuf.descriptor_malformed")?,
            (4, 2) => {
                let nested = length_delimited(&mut cursor, "protobuf.descriptor_malformed")?;
                file.messages.push(parse_message(nested, 1)?);
            }
            (5, 2) => {
                let nested = length_delimited(&mut cursor, "protobuf.descriptor_malformed")?;
                file.enums.push(parse_enum(nested)?);
            }
            (7, 2) => {
                let nested = length_delimited(&mut cursor, "protobuf.descriptor_malformed")?;
                file.extensions.push(parse_field(nested)?);
            }
            (12, 2) => {
                let syntax = string(&mut cursor, "protobuf.descriptor_malformed")?;
                file.syntax = match syntax.as_str() {
                    "proto2" => ProtobufSyntax::Proto2,
                    "proto3" => ProtobufSyntax::Proto3,
                    _ => ProtobufSyntax::Unknown,
                };
            }
            (14, 0) => {
                cursor.read_varint("protobuf.descriptor_malformed")?;
                edition_present = true;
            }
            _ => skip_field(&mut cursor, wire, number, "protobuf.descriptor_malformed")?,
        }
    }
    if edition_present {
        file.syntax = ProtobufSyntax::Editions;
    }
    if file.name.is_empty() {
        file.name = "<unnamed>".to_string();
    }
    Ok(file)
}

fn parse_message(bytes: &[u8], depth: usize) -> Result<MessageProto, DecodeError> {
    if depth > 128 {
        return Err(DecodeError::new(
            "protobuf.descriptor_nesting_limit",
            "descriptor message nesting exceeds 128",
            0,
        ));
    }
    let mut cursor = Cursor::new(bytes);
    let mut message = MessageProto::default();
    while !cursor.is_empty() {
        let (number, wire, _) = key(&mut cursor, "protobuf.descriptor_malformed")?;
        match (number, wire) {
            (1, 2) => message.name = string(&mut cursor, "protobuf.descriptor_malformed")?,
            (2, 2) => {
                let nested = length_delimited(&mut cursor, "protobuf.descriptor_malformed")?;
                message.fields.push(parse_field(nested)?);
            }
            (3, 2) => {
                let nested = length_delimited(&mut cursor, "protobuf.descriptor_malformed")?;
                message.nested.push(parse_message(nested, depth + 1)?);
            }
            (4, 2) => {
                let nested = length_delimited(&mut cursor, "protobuf.descriptor_malformed")?;
                message.enums.push(parse_enum(nested)?);
            }
            (6, 2) => {
                let nested = length_delimited(&mut cursor, "protobuf.descriptor_malformed")?;
                message.extensions.push(parse_field(nested)?);
            }
            (7, 2) => {
                let options = length_delimited(&mut cursor, "protobuf.descriptor_malformed")?;
                message.map_entry = parse_message_options(options)?;
            }
            (8, 2) => {
                let oneof = length_delimited(&mut cursor, "protobuf.descriptor_malformed")?;
                message.oneofs.push(parse_oneof(oneof)?);
            }
            _ => skip_field(&mut cursor, wire, number, "protobuf.descriptor_malformed")?,
        }
    }
    if message.name.is_empty() {
        return Err(DecodeError::new(
            "protobuf.descriptor_missing_name",
            "message descriptor has no name",
            0,
        ));
    }
    Ok(message)
}

fn parse_field(bytes: &[u8]) -> Result<FieldProto, DecodeError> {
    let mut cursor = Cursor::new(bytes);
    let mut field = FieldProto::default();
    while !cursor.is_empty() {
        let (number, wire, _) = key(&mut cursor, "protobuf.descriptor_malformed")?;
        match (number, wire) {
            (1, 2) => field.name = string(&mut cursor, "protobuf.descriptor_malformed")?,
            (2, 2) => field.extendee = Some(string(&mut cursor, "protobuf.descriptor_malformed")?),
            (3, 0) => field.number = cursor.read_varint("protobuf.descriptor_malformed")? as i32,
            (4, 0) => field.label = cursor.read_varint("protobuf.descriptor_malformed")? as i32,
            (5, 0) => field.kind = cursor.read_varint("protobuf.descriptor_malformed")? as i32,
            (6, 2) => field.type_name = Some(string(&mut cursor, "protobuf.descriptor_malformed")?),
            (8, 2) => {
                let options = length_delimited(&mut cursor, "protobuf.descriptor_malformed")?;
                field.packed = parse_field_options(options)?;
            }
            (9, 0) => {
                field.oneof_index = Some(
                    usize::try_from(cursor.read_varint("protobuf.descriptor_malformed")?).map_err(
                        |_| {
                            DecodeError::new(
                                "protobuf.descriptor_invalid_oneof",
                                "oneof index exceeds usize",
                                0,
                            )
                        },
                    )?,
                )
            }
            (10, 2) => field.json_name = string(&mut cursor, "protobuf.descriptor_malformed")?,
            _ => skip_field(&mut cursor, wire, number, "protobuf.descriptor_malformed")?,
        }
    }
    Ok(field)
}

fn parse_enum(bytes: &[u8]) -> Result<EnumProto, DecodeError> {
    let mut cursor = Cursor::new(bytes);
    let mut descriptor = EnumProto::default();
    while !cursor.is_empty() {
        let (number, wire, _) = key(&mut cursor, "protobuf.descriptor_malformed")?;
        match (number, wire) {
            (1, 2) => descriptor.name = string(&mut cursor, "protobuf.descriptor_malformed")?,
            (2, 2) => {
                let value = length_delimited(&mut cursor, "protobuf.descriptor_malformed")?;
                descriptor.values.push(parse_enum_value(value)?);
            }
            _ => skip_field(&mut cursor, wire, number, "protobuf.descriptor_malformed")?,
        }
    }
    if descriptor.name.is_empty() {
        return Err(DecodeError::new(
            "protobuf.descriptor_missing_name",
            "enum descriptor has no name",
            0,
        ));
    }
    Ok(descriptor)
}

fn parse_enum_value(bytes: &[u8]) -> Result<(String, i32), DecodeError> {
    let mut cursor = Cursor::new(bytes);
    let mut name = String::new();
    let mut value = 0i32;
    while !cursor.is_empty() {
        let (number, wire, _) = key(&mut cursor, "protobuf.descriptor_malformed")?;
        match (number, wire) {
            (1, 2) => name = string(&mut cursor, "protobuf.descriptor_malformed")?,
            (2, 0) => value = cursor.read_varint("protobuf.descriptor_malformed")? as i32,
            _ => skip_field(&mut cursor, wire, number, "protobuf.descriptor_malformed")?,
        }
    }
    if name.is_empty() {
        return Err(DecodeError::new(
            "protobuf.descriptor_missing_name",
            "enum value has no name",
            0,
        ));
    }
    Ok((name, value))
}

fn parse_oneof(bytes: &[u8]) -> Result<String, DecodeError> {
    let mut cursor = Cursor::new(bytes);
    let mut name = String::new();
    while !cursor.is_empty() {
        let (number, wire, _) = key(&mut cursor, "protobuf.descriptor_malformed")?;
        if number == 1 && wire == 2 {
            name = string(&mut cursor, "protobuf.descriptor_malformed")?;
        } else {
            skip_field(&mut cursor, wire, number, "protobuf.descriptor_malformed")?;
        }
    }
    Ok(name)
}

fn parse_message_options(bytes: &[u8]) -> Result<bool, DecodeError> {
    let mut cursor = Cursor::new(bytes);
    let mut map_entry = false;
    while !cursor.is_empty() {
        let (number, wire, _) = key(&mut cursor, "protobuf.descriptor_malformed")?;
        if number == 7 && wire == 0 {
            map_entry = cursor.read_varint("protobuf.descriptor_malformed")? != 0;
        } else {
            skip_field(&mut cursor, wire, number, "protobuf.descriptor_malformed")?;
        }
    }
    Ok(map_entry)
}

fn parse_field_options(bytes: &[u8]) -> Result<Option<bool>, DecodeError> {
    let mut cursor = Cursor::new(bytes);
    let mut packed = None;
    while !cursor.is_empty() {
        let (number, wire, _) = key(&mut cursor, "protobuf.descriptor_malformed")?;
        if number == 2 && wire == 0 {
            packed = Some(cursor.read_varint("protobuf.descriptor_malformed")? != 0);
        } else {
            skip_field(&mut cursor, wire, number, "protobuf.descriptor_malformed")?;
        }
    }
    Ok(packed)
}

fn convert_field(
    field: FieldProto,
    syntax: ProtobufSyntax,
    scope: &str,
    oneofs: &[String],
    extension: bool,
) -> Result<FieldDescriptor, DecodeError> {
    if field.name.is_empty() || field.number <= 0 {
        return Err(DecodeError::new(
            "protobuf.descriptor_invalid_field",
            "field descriptors require a name and positive number",
            0,
        ));
    }
    let number = u32::try_from(field.number).map_err(|_| {
        DecodeError::new(
            "protobuf.descriptor_invalid_field",
            format!("field {} has invalid number {}", field.name, field.number),
            0,
        )
    })?;
    if number > 536_870_911 || (19_000..=19_999).contains(&number) {
        return Err(DecodeError::new(
            "protobuf.descriptor_invalid_field",
            format!(
                "field {} uses reserved or out-of-range number {number}",
                field.name
            ),
            0,
        ));
    }
    let label = match field.label {
        1 | 0 => FieldLabel::Optional,
        2 => FieldLabel::Required,
        3 => FieldLabel::Repeated,
        value => {
            return Err(DecodeError::new(
                "protobuf.descriptor_invalid_field",
                format!("field {} has unknown label {value}", field.name),
                0,
            ));
        }
    };
    let kind = match field.kind {
        1 => FieldKind::Double,
        2 => FieldKind::Float,
        3 => FieldKind::Int64,
        4 => FieldKind::Uint64,
        5 => FieldKind::Int32,
        6 => FieldKind::Fixed64,
        7 => FieldKind::Fixed32,
        8 => FieldKind::Bool,
        9 => FieldKind::String,
        10 => FieldKind::Group,
        11 => FieldKind::Message,
        12 => FieldKind::Bytes,
        13 => FieldKind::Uint32,
        14 => FieldKind::Enum,
        15 => FieldKind::Sfixed32,
        16 => FieldKind::Sfixed64,
        17 => FieldKind::Sint32,
        18 => FieldKind::Sint64,
        value => {
            return Err(DecodeError::new(
                "protobuf.descriptor_invalid_field",
                format!("field {} has unknown type {value}", field.name),
                0,
            ));
        }
    };
    if matches!(
        kind,
        FieldKind::Message | FieldKind::Group | FieldKind::Enum
    ) && field.type_name.as_deref().is_none_or(str::is_empty)
    {
        return Err(DecodeError::new(
            "protobuf.descriptor_invalid_field",
            format!("field {} requires a type name", field.name),
            0,
        ));
    }
    let oneof = field
        .oneof_index
        .map(|index| {
            oneofs.get(index).cloned().ok_or_else(|| {
                DecodeError::new(
                    "protobuf.descriptor_invalid_oneof",
                    format!(
                        "field {} references missing oneof index {index}",
                        field.name
                    ),
                    0,
                )
            })
        })
        .transpose()?;
    let packed_default = label == FieldLabel::Repeated
        && kind.packable()
        && matches!(syntax, ProtobufSyntax::Proto3 | ProtobufSyntax::Editions);
    let json_name = if field.json_name.is_empty() {
        lower_camel_case(&field.name)
    } else {
        field.json_name
    };
    Ok(FieldDescriptor {
        name: field.name,
        json_name,
        number,
        label,
        kind,
        type_name: field
            .type_name
            .as_deref()
            .map(|name| normalize_type_name(name, scope)),
        packed: field.packed.unwrap_or(packed_default),
        extension,
        oneof,
    })
}

fn normalize_type_name(name: &str, scope: &str) -> String {
    if let Some(name) = name.strip_prefix('.') {
        name.to_string()
    } else if name.contains('.') || scope.is_empty() {
        name.to_string()
    } else {
        let parent = scope.rsplit_once('.').map_or(scope, |(parent, _)| parent);
        join_name(parent, name)
    }
}

fn join_name(scope: &str, name: &str) -> String {
    match (scope.trim_matches('.'), name.trim_matches('.')) {
        ("", name) => name.to_string(),
        (scope, name) => format!("{scope}.{name}"),
    }
}

fn lower_camel_case(name: &str) -> String {
    let mut result = String::with_capacity(name.len());
    let mut uppercase = false;
    for (index, character) in name.chars().enumerate() {
        if character == '_' {
            uppercase = true;
        } else if uppercase {
            result.extend(character.to_uppercase());
            uppercase = false;
        } else if index == 0 {
            result.extend(character.to_lowercase());
        } else {
            result.push(character);
        }
    }
    result
}

fn key(cursor: &mut Cursor<'_>, code: &'static str) -> Result<(u32, u8, usize), DecodeError> {
    let start = cursor.position();
    let key = cursor.read_varint(code)?;
    let number = u32::try_from(key >> 3)
        .map_err(|_| DecodeError::new(code, "field number exceeds u32", start))?;
    let wire = (key & 7) as u8;
    if number == 0 {
        return Err(DecodeError::new(
            code,
            "field number zero is invalid",
            start,
        ));
    }
    if wire > 5 {
        return Err(DecodeError::new(
            code,
            format!("wire type {wire} is reserved"),
            start,
        ));
    }
    Ok((number, wire, start))
}

fn length_delimited<'a>(
    cursor: &mut Cursor<'a>,
    code: &'static str,
) -> Result<&'a [u8], DecodeError> {
    let offset = cursor.position();
    let length = usize::try_from(cursor.read_varint(code)?)
        .map_err(|_| DecodeError::new(code, "length exceeds usize", offset))?;
    cursor.take(length, code)
}

fn string(cursor: &mut Cursor<'_>, code: &'static str) -> Result<String, DecodeError> {
    let offset = cursor.position();
    let bytes = length_delimited(cursor, code)?;
    std::str::from_utf8(bytes)
        .map(str::to_string)
        .map_err(|error| {
            DecodeError::new(
                code,
                format!("descriptor string is not UTF-8: {error}"),
                offset,
            )
        })
}

fn skip_field(
    cursor: &mut Cursor<'_>,
    wire: u8,
    field_number: u32,
    code: &'static str,
) -> Result<(), DecodeError> {
    match wire {
        0 => {
            cursor.read_varint(code)?;
        }
        1 => {
            cursor.take(8, code)?;
        }
        2 => {
            let length = usize::try_from(cursor.read_varint(code)?).map_err(|_| {
                DecodeError::new(
                    code,
                    "length-delimited field exceeds usize",
                    cursor.position(),
                )
            })?;
            cursor.take(length, code)?;
        }
        3 => loop {
            let (nested_number, nested_wire, _) = key(cursor, code)?;
            if nested_wire == 4 {
                if nested_number != field_number {
                    return Err(DecodeError::new(
                        code,
                        "group ended with a different field number",
                        cursor.position(),
                    ));
                }
                break;
            }
            skip_field(cursor, nested_wire, nested_number, code)?;
        },
        4 => {
            return Err(DecodeError::new(
                code,
                "unexpected end-group marker",
                cursor.position(),
            ));
        }
        5 => {
            cursor.take(4, code)?;
        }
        _ => {
            return Err(DecodeError::new(
                code,
                format!("reserved wire type {wire}"),
                cursor.position(),
            ));
        }
    }
    Ok(())
}
