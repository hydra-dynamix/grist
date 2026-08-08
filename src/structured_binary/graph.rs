use super::{BinaryScalar, BinaryValue, BinaryValueKind, StructuredBinaryDocument};
use crate::core::SchemaVersion;
use crate::document_graph::{
    DocumentGraph, DocumentGraphContext, DocumentKind, DocumentNode, DocumentNodeKind,
    GraphIdGenerator, ProjectionAddress, ToDocumentGraph, TransformError,
};

impl ToDocumentGraph for StructuredBinaryDocument {
    fn to_document_graph(
        &self,
        context: DocumentGraphContext,
    ) -> Result<DocumentGraph, TransformError> {
        let identities = context
            .identity_generator(SchemaVersion::STRUCTURED_BINARY_V1, "structured-binary")
            .map_err(transform_error)?;
        let mut graph = DocumentGraph::new(context.graph_id, DocumentKind::StructuredBinary)
            .with_projection(
                "structured-binary",
                SchemaVersion::STRUCTURED_BINARY_V1,
                "grist.structured-binary.to-document-graph.v1",
            );
        graph.source = context.source;
        graph.language = Some(format!("{:?}", self.format).to_ascii_lowercase());
        graph.dialect = context.dialect;
        graph.attrs = context.attrs;
        graph.attrs.insert(
            "schema_identity".to_string(),
            serde_json::to_value(&self.schema_identity).map_err(transform_error)?,
        );
        let root_id = node_id(&identities, &["document"], "root", None)?;
        graph.add_node(DocumentNode::new(&root_id, DocumentNodeKind::Document).with_ordinal(0));
        for record in &self.records {
            let record_id = node_id(
                &identities,
                &["document", "records", &record.index.to_string()],
                &format!("record:{}", record.index),
                Some(record.locator.clone()),
            )?;
            let mut record_node = DocumentNode::new(&record_id, DocumentNodeKind::Record)
                .with_locator(record.locator.clone())
                .with_ordinal(record.index - 1)
                .with_attr("byte_start", record.byte_start)
                .with_attr("byte_end", record.byte_end);
            record_node.extensions.insert(
                "grist.structured_binary".to_string(),
                serde_json::json!({
                    "format": self.format,
                    "record_index": record.index,
                }),
            );
            graph.add_node(record_node);
            graph.add_contains(&root_id, &record_id);
            add_value(
                &mut graph,
                &identities,
                &record_id,
                &record.value,
                vec![
                    "document".to_string(),
                    "records".to_string(),
                    record.index.to_string(),
                ],
                0,
            )?;
        }
        graph
            .finalize_projection(&identities)
            .map_err(transform_error)?;
        Ok(graph)
    }
}

fn add_value(
    graph: &mut DocumentGraph,
    identities: &GraphIdGenerator,
    parent: &str,
    value: &BinaryValue,
    mut structural_path: Vec<String>,
    ordinal: usize,
) -> Result<String, TransformError> {
    structural_path.push("value".to_string());
    let id = node_id(
        identities,
        &structural_path
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        &value.id,
        Some(value.locator.clone()),
    )?;
    let kind = if value.kind == BinaryValueKind::Unknown {
        DocumentNodeKind::Raw
    } else {
        DocumentNodeKind::StructuredValue
    };
    let mut node = DocumentNode::new(&id, kind)
        .with_locator(value.locator.clone())
        .with_ordinal(ordinal)
        .with_attr(
            "value_kind",
            serde_json::to_value(value.kind).map_err(transform_error)?,
        )
        .with_attr("path", value.path.clone())
        .with_attr("byte_start", value.byte_start)
        .with_attr("byte_end", value.byte_end)
        .with_attr("recovered", value.recovered);
    node.text = value.scalar.as_ref().map(scalar_text);
    node.name = value
        .protobuf_field
        .as_ref()
        .map(|field| field.field_name.clone())
        .or_else(|| value.cbor_tag.map(|tag| format!("tag {tag}")))
        .or_else(|| {
            value
                .messagepack_extension
                .as_ref()
                .map(|extension| format!("extension {}", extension.type_code))
        });
    node.extensions.insert(
        "grist.structured_binary".to_string(),
        serde_json::json!({
            "cbor_tag": value.cbor_tag,
            "messagepack_extension": value.messagepack_extension,
            "protobuf_field": value.protobuf_field,
            "indefinite": value.indefinite,
        }),
    );
    graph.add_node(node);
    graph.add_contains(parent, &id);

    for entry in &value.entries {
        let field_path = [
            structural_path.clone(),
            vec!["entries".to_string(), entry.index.to_string()],
        ]
        .concat();
        let field_locator = entry
            .key
            .as_deref()
            .map_or_else(|| entry.value.locator.clone(), |key| key.locator.clone());
        let field_native = entry.field_number.map_or_else(
            || format!("entry:{}", entry.index),
            |number| format!("field:{number}:{}", entry.index),
        );
        let field_id = node_id(
            identities,
            &field_path.iter().map(String::as_str).collect::<Vec<_>>(),
            &field_native,
            Some(field_locator.clone()),
        )?;
        let mut field_node = DocumentNode::new(&field_id, DocumentNodeKind::Field)
            .with_locator(field_locator)
            .with_ordinal(entry.index)
            .with_attr("duplicate_ordinal", entry.duplicate_ordinal);
        field_node.name = entry.field_name.clone().or_else(|| {
            entry.key.as_deref().and_then(|key| match &key.scalar {
                Some(BinaryScalar::Text { value }) => Some(value.clone()),
                _ => None,
            })
        });
        field_node.text = field_node.name.clone();
        field_node.extensions.insert(
            "grist.structured_binary".to_string(),
            serde_json::json!({
                "field_number": entry.field_number,
                "key": entry.key.as_deref().map(BinaryValue::json_projection),
            }),
        );
        graph.add_node(field_node);
        graph.add_contains(&id, &field_id);
        add_value(graph, identities, &field_id, &entry.value, field_path, 0)?;
    }
    for (index, item) in value.items.iter().enumerate() {
        let item_path = [
            structural_path.clone(),
            vec!["items".to_string(), index.to_string()],
        ]
        .concat();
        add_value(graph, identities, &id, item, item_path, index)?;
    }
    Ok(id)
}

fn node_id(
    identities: &GraphIdGenerator,
    path: &[&str],
    native_id: &str,
    locator: Option<crate::core::SourceLocator>,
) -> Result<String, TransformError> {
    let mut address = ProjectionAddress::native(path.iter().copied(), native_id);
    if let Some(locator) = locator {
        address = address.with_locator(locator);
    }
    identities.node_id(&address).map_err(transform_error)
}

fn scalar_text(scalar: &BinaryScalar) -> String {
    match scalar {
        BinaryScalar::Null => "null".to_string(),
        BinaryScalar::Undefined => "undefined".to_string(),
        BinaryScalar::Boolean { value } => value.to_string(),
        BinaryScalar::Integer { canonical } | BinaryScalar::Float { canonical, .. } => {
            canonical.clone()
        }
        BinaryScalar::Text { value } => value.clone(),
        BinaryScalar::Bytes { hex, .. } => hex.clone(),
        BinaryScalar::Simple { value } => value.to_string(),
        BinaryScalar::Enum {
            number,
            name: Some(name),
        } => format!("{name} ({number})"),
        BinaryScalar::Enum { number, name: None } => number.to_string(),
    }
}

fn transform_error(error: impl std::fmt::Display) -> TransformError {
    TransformError::Other {
        message: error.to_string(),
    }
}
