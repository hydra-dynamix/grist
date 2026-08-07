use grist::core::{
    BoundingBox, CellAddress, CoordinateOrigin, CoordinateUnit, DerivedNodeReference, IndexBase,
    IndexPosition, IndexRange, LineIndex, LocationComponent, LocatorConfidence, LocatorPrecision,
    SchemaVersion, SourceLocator, SourceRange,
};

fn one(value: u64) -> IndexPosition {
    IndexPosition::one_based(value).unwrap()
}

fn box_region() -> BoundingBox {
    BoundingBox {
        x: 0.1,
        y: 0.2,
        width: 0.3,
        height: 0.4,
        unit: CoordinateUnit::Normalized,
        origin: CoordinateOrigin::TopLeft,
    }
}

fn every_component() -> Vec<LocationComponent> {
    vec![
        SourceRange {
            byte_start: 4,
            byte_end: 9,
            start_line: 2,
            start_column: 1,
            end_line: 2,
            end_column: 6,
        }
        .into(),
        LocationComponent::PdfRegion {
            page: one(3),
            bbox: Some(box_region()),
            rotation_degrees: Some(90),
            tokens: Some(IndexRange::new(0, 7, IndexBase::Zero).unwrap()),
        },
        LocationComponent::OoxmlPart {
            part: "/word/document.xml".into(),
            paragraph: Some(one(4)),
            run: Some(one(2)),
            table: None,
            row: None,
            column: None,
            object_id: Some("bookmark-7".into()),
        },
        LocationComponent::SlideRegion {
            slide: one(8),
            shape_id: Some("shape-14".into()),
            bbox: Some(box_region()),
        },
        LocationComponent::SheetRange {
            sheet: "Summary".into(),
            start_cell: CellAddress::a1(4, 2).unwrap(),
            end_cell: CellAddress::a1(9, 5).unwrap(),
        },
        LocationComponent::NotebookCell {
            index: IndexPosition::zero_based(3),
            cell_id: Some("cell-a".into()),
            output_index: Some(IndexPosition::zero_based(1)),
        },
        LocationComponent::EmailPart {
            message_id: Some("<message@example.test>".into()),
            mime_path: vec![one(2), one(1)],
            header: None,
        },
        LocationComponent::ArchiveMember {
            member_path: "mail/message.eml".into(),
            member_index: IndexPosition::zero_based(5),
        },
        LocationComponent::ImageRegion {
            frame: one(2),
            bbox: Some(box_region()),
        },
        LocationComponent::MediaTime {
            start_ms: 1_250,
            end_ms: 4_500,
            track: Some(IndexPosition::zero_based(0)),
        },
        LocationComponent::RecordRange {
            collection: "events".into(),
            records: IndexRange::new(1, 4, IndexBase::One).unwrap(),
            field: Some("payload".into()),
        },
        LocationComponent::JsonPointer {
            pointer: "/items/0/a~1b".into(),
        },
        LocationComponent::XmlPath {
            path: "/article/body/sec[2]/p[1]".into(),
        },
    ]
}

#[test]
fn every_tagged_component_round_trips() {
    for component in every_component() {
        let expected = SourceLocator::exact(component).unwrap();
        let json = serde_json::to_string(&expected).unwrap();
        let actual: SourceLocator = serde_json::from_str(&json).unwrap();
        assert_eq!(actual, expected, "failed round-trip for {json}");
    }
}

#[test]
fn containment_order_is_outermost_to_innermost_and_survives_json() {
    let locator = SourceLocator::exact(LocationComponent::ArchiveMember {
        member_path: "messages.zip".into(),
        member_index: one(1),
    })
    .unwrap()
    .nested(LocationComponent::EmailPart {
        message_id: Some("<nested@example.test>".into()),
        mime_path: vec![one(2)],
        header: None,
    })
    .unwrap()
    .nested(LocationComponent::OoxmlPart {
        part: "/word/document.xml".into(),
        paragraph: Some(one(3)),
        run: Some(one(1)),
        table: None,
        row: None,
        column: None,
        object_id: None,
    })
    .unwrap()
    .nested(SourceRange {
        byte_start: 12,
        byte_end: 18,
        start_line: 1,
        start_column: 13,
        end_line: 1,
        end_column: 19,
    })
    .unwrap();

    let value = serde_json::to_value(&locator).unwrap();
    let types: Vec<_> = value["components"]
        .as_array()
        .unwrap()
        .iter()
        .map(|component| component["type"].as_str().unwrap())
        .collect();
    assert_eq!(
        types,
        ["archive_member", "email_part", "ooxml_part", "text_range"]
    );
    assert!(matches!(
        locator.innermost(),
        LocationComponent::TextRange { .. }
    ));
    assert_eq!(
        serde_json::from_value::<SourceLocator>(value).unwrap(),
        locator
    );
}

#[test]
fn text_offsets_are_zero_based_half_open_and_human_positions_are_one_based() {
    let text = "é🙂\nβ";
    let index = LineIndex::new(text);
    let emoji = SourceRange::new(2, 6, &index);
    assert_eq!((emoji.byte_start, emoji.byte_end), (2, 6));
    assert_eq!(
        (
            emoji.start_line,
            emoji.start_column,
            emoji.end_line,
            emoji.end_column,
        ),
        (1, 2, 1, 3)
    );
    let beta = SourceRange::new(7, text.len(), &index);
    assert_eq!(
        (
            beta.start_line,
            beta.start_column,
            beta.end_line,
            beta.end_column,
        ),
        (2, 1, 2, 2)
    );
}

#[test]
fn index_bases_and_half_open_ranges_are_enforced_on_deserialization() {
    assert!(IndexPosition::one_based(0).is_err());
    assert!(IndexRange::new(4, 3, IndexBase::Zero).is_err());

    let invalid_page = serde_json::json!({
        "components": [{
            "type": "pdf_region",
            "page": {"value": 0, "base": "one"}
        }],
        "precision": "exact"
    });
    assert!(serde_json::from_value::<SourceLocator>(invalid_page).is_err());

    let mixed_cells = SourceLocator::exact(LocationComponent::SheetRange {
        sheet: "Data".into(),
        start_cell: CellAddress {
            row: 1,
            column: 1,
            base: IndexBase::One,
        },
        end_cell: CellAddress {
            row: 1,
            column: 1,
            base: IndexBase::Zero,
        },
    });
    assert!(mixed_cells.is_err());
}

#[test]
fn approximate_and_synthetic_locations_have_required_evidence() {
    let approximate = SourceLocator::approximate(
        LocationComponent::ImageRegion {
            frame: one(1),
            bbox: Some(box_region()),
        },
        LocatorConfidence::new(0.82).unwrap(),
    )
    .unwrap();
    assert_eq!(approximate.precision().name(), "approximate");
    assert_eq!(approximate.precision().confidence().unwrap().get(), 0.82);
    let approximate_json = serde_json::to_value(&approximate).unwrap();
    assert_eq!(approximate_json["precision"], "approximate");
    assert_eq!(approximate_json["confidence"], 0.82);

    let derived = DerivedNodeReference::new(
        vec!["node:page-1-image".into()],
        vec!["ocr-provider:acme/v3".into(), "layout-reconcile:v1".into()],
    )
    .unwrap();
    let synthetic = SourceLocator::synthetic(
        LocationComponent::JsonPointer {
            pointer: "/normalized_text".into(),
        },
        derived.clone(),
    )
    .unwrap();
    assert_eq!(synthetic.precision().name(), "synthetic");
    assert_eq!(synthetic.precision().derived_from(), Some(&derived));

    let missing_confidence = serde_json::json!({
        "components": [{"type": "xml_path", "path": "/article"}],
        "precision": "approximate"
    });
    assert!(serde_json::from_value::<SourceLocator>(missing_confidence).is_err());

    let missing_derivation = serde_json::json!({
        "components": [{"type": "json_pointer", "pointer": ""}],
        "precision": "synthetic"
    });
    assert!(serde_json::from_value::<SourceLocator>(missing_derivation).is_err());
    assert!(LocatorConfidence::new(1.01).is_err());
}

#[test]
fn locator_schema_exposes_all_tags_precision_and_confidence_bounds() {
    let entry = grist::schema::list_schemas()
        .into_iter()
        .find(|entry| entry.name == "source-locator")
        .unwrap();
    assert_eq!(entry.schema_version, SchemaVersion::SOURCE_LOCATOR_V1);

    let schema = grist::schema::schema_json("source-locator").unwrap();
    let schema_text = serde_json::to_string(&schema).unwrap();
    for tag in [
        "text_range",
        "pdf_region",
        "ooxml_part",
        "slide_region",
        "sheet_range",
        "notebook_cell",
        "email_part",
        "archive_member",
        "image_region",
        "media_time",
        "record_range",
        "json_pointer",
        "xml_path",
        "exact",
        "approximate",
        "synthetic",
    ] {
        assert!(schema_text.contains(tag), "schema omitted {tag}");
    }
    let confidence = &schema["definitions"]["LocatorConfidence"];
    assert_eq!(confidence["minimum"], 0.0);
    assert_eq!(confidence["maximum"], 1.0);

    let validator = jsonschema::validator_for(&schema).unwrap();
    for component in every_component() {
        let locator = SourceLocator::exact(component).unwrap();
        let value = serde_json::to_value(locator).unwrap();
        assert!(
            validator.is_valid(&value),
            "generated schema rejected {value}"
        );
    }
}

#[test]
fn malformed_component_semantics_are_rejected() {
    for value in [
        serde_json::json!({
            "components": [],
            "precision": "exact"
        }),
        serde_json::json!({
            "components": [{"type": "json_pointer", "pointer": "/bad~2escape"}],
            "precision": "exact"
        }),
        serde_json::json!({
            "components": [{"type": "xml_path", "path": "relative/path"}],
            "precision": "exact"
        }),
        serde_json::json!({
            "components": [{
                "type": "media_time",
                "start_ms": 20,
                "end_ms": 10
            }],
            "precision": "exact"
        }),
    ] {
        assert!(
            serde_json::from_value::<SourceLocator>(value.clone()).is_err(),
            "accepted invalid locator {value}"
        );
    }

    let invalid_derivation = SourceLocator::new(
        vec![LocationComponent::JsonPointer {
            pointer: String::new(),
        }],
        LocatorPrecision::Synthetic {
            derived_from: DerivedNodeReference {
                source_node_ids: vec![],
                derivation_steps: vec![],
            },
            confidence: None,
        },
    );
    assert!(invalid_derivation.is_err());
}
