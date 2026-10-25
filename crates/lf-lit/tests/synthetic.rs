//! Documents built in code, so these tests need no sample files.

use lf_lit::{Container, Document, Header, Object, Reference, State, Values};
use uuid::Uuid;

const PROJECT: Uuid = Uuid::from_u128(0x3690a615_9a64_4bc2_8a3d_b31a29c3f2ba);
const PROPERTY: Uuid = Uuid::from_u128(0x7c0b2130_71dd_44f9_81f2_3fbfb860fb5b);
const VECTOR2: Uuid = Uuid::from_u128(0x754b9c0d_a4c2_406f_9a42_425f8122e2c2);
const STRING: Uuid = Uuid::from_u128(0xef3795fb_56ff_430f_96cf_6b8076cd6f8f);
const BOOLEAN: Uuid = Uuid::from_u128(0x272847ea_0362_4003_9bba_6cbfa7b23520);
const DURATION: Uuid = Uuid::from_u128(0xf5587d3a_5aa3_4fe0_a518_3ed9affbbde0);
const ENUM: Uuid = Uuid::from_u128(0x865db8a2_ad30_477a_aa56_4becae810f9d);

fn id(n: u128) -> Uuid {
    Uuid::from_u128(0x1000 + n)
}

fn controller(n: u128, class_id: Uuid, owner: Uuid, state: State) -> Object {
    Object {
        id: id(n),
        class_id,
        owner_id: Some(owner),
        name: None,
        state: Some(state),
        references: None,
    }
}

/// A small v23-shaped graph: a project whose slots point straight at value
/// controllers owned by elided Property objects, plus one dynamic property.
fn sample() -> Document {
    let project = Object {
        id: id(0),
        class_id: PROJECT,
        owner_id: None,
        name: Some("Project".into()),
        state: None,
        references: Some(
            (1..=5)
                .map(|n| Reference {
                    local_id: n as u32 - 1,
                    id: id(n),
                })
                .collect(),
        ),
    };
    let points = State {
        is_array: Some(true),
        count: Some(2),
        values: Some(Values::F32(vec![1.5, 2.5, -3.0, 4.0])),
        ..State::default()
    };
    let shader_input = Object {
        id: id(6),
        class_id: PROPERTY,
        owner_id: Some(id(0)),
        name: Some("speed".into()),
        state: Some(State {
            dynamic_description: Some(vec![
                ("name".into(), "speed".into()),
                ("value-type".into(), "FLOAT".into()),
                ("default-value".into(), "0.5".into()),
            ]),
            ..State::default()
        }),
        references: None,
    };
    let objects = vec![
        project,
        controller(1, VECTOR2, id(101), points),
        controller(
            2,
            STRING,
            id(102),
            State {
                values: Some(Values::Strings(vec!["café".into()])),
                ..State::default()
            },
        ),
        controller(
            3,
            BOOLEAN,
            id(103),
            State {
                values: Some(Values::Bool(vec![1])),
                ..State::default()
            },
        ),
        controller(
            4,
            DURATION,
            id(104),
            State {
                values: Some(Values::Ticks(vec![7_056_000_000])),
                ..State::default()
            },
        ),
        controller(
            5,
            ENUM,
            id(105),
            State {
                subtype_id: Some(id(200)),
                values: Some(Values::Strings(vec!["UNION".into()])),
                ..State::default()
            },
        ),
        shader_input,
    ];
    Document::new(
        Header {
            format_version: 1,
            max_bytes_per_line: Some(0xFFFF_FFFF),
            blob_text_format: None,
            data_version: 23,
            original_data_version: 23,
        },
        objects,
    )
}

#[test]
fn binary_round_trip() {
    let doc = sample();
    let bytes = doc.to_binary().unwrap();
    let read = Document::read(&bytes).unwrap();
    assert_eq!(read.container, Container::Binary);
    assert_eq!(read.header, doc.header);
    assert_eq!(read.objects, doc.objects);
    assert_eq!(
        read.to_binary().unwrap(),
        bytes,
        "rewriting a read document is byte-identical"
    );
}

#[test]
fn every_uuid_is_interned_once() {
    let read = Document::read(&sample().to_binary().unwrap()).unwrap();
    let table = read.id_table();
    let mut unique = table.to_vec();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), table.len());
    // 7 objects, 7 classes, 5 elided property owners, 1 enum subtype.
    assert_eq!(table.len(), 7 + 7 + 5 + 1);
}

#[test]
fn appended_objects_extend_the_id_table() {
    let mut read = Document::read(&sample().to_binary().unwrap()).unwrap();
    let before = read.id_table().to_vec();
    read.objects
        .push(controller(7, STRING, id(0), State::default()));
    let reread = Document::read(&read.to_binary().unwrap()).unwrap();
    assert_eq!(&reread.id_table()[..before.len()], &before[..]);
    assert_eq!(reread.id_table()[before.len()], id(7));
}

#[test]
fn rejects_count_that_disagrees_with_values() {
    let mut doc = sample();
    doc.objects[1].state.as_mut().unwrap().count = Some(3);
    let err = Document::read(&doc.to_binary().unwrap()).unwrap_err();
    assert!(err.to_string().contains("count is 3"), "{err}");
}

#[test]
fn reads_json_container_with_trailing_commas() {
    let json = format!(
        r#"{{"format":"JSON Document","formatVersion":1,"blobTextFormat":"byte-array",
        "header|8e73ed04-344f-45f0-8821-85fa77dcb9b5":{{"dataVersionNumber":9,"originalDataVersionNumber":9,}},
        "objects|54fa5fd0-d3cb-472f-b660-3bb67f89eef4":{{"count":2,
          "0":{{"name":"Project","id":"{p}","classId":"{project}",
               "references|95623259-f9de-4a79-922b-e4eef38436dd":{{"count":1,"0":{{"localId":0,"id":"{c}"}},}},}},
          "1":{{"name":"Project_duration_Property_ValueController","id":"{c}","classId":"{duration}","ownerId":"{p}",
               "state|0c5a8d0c-d533-47b7-bc25-4e21443714e1":{{"isArray":false,"values":["7056000000flk",],}},}},
        }},}}"#,
        p = id(0),
        c = id(1),
        project = PROJECT,
        duration = DURATION,
    );
    let doc = Document::read(json.as_bytes()).unwrap();
    assert_eq!(doc.container, Container::Json);
    assert_eq!(doc.header.data_version, 9);
    assert_eq!(doc.objects.len(), 2);
    let state = doc.objects[1].state.as_ref().unwrap();
    assert_eq!(state.values, Some(Values::Ticks(vec![7_056_000_000])));
    assert_eq!(doc.objects[0].references.as_ref().unwrap()[0].id, id(1));

    let rewritten = Document::read(&doc.to_binary().unwrap()).unwrap();
    assert_eq!(rewritten.objects, doc.objects);
}

#[test]
fn rejects_unknown_fields() {
    let json = r#"{"format":"JSON Document","formatVersion":1,"surprise":true}"#;
    let err = Document::read(json.as_bytes()).unwrap_err();
    assert!(err.to_string().contains("surprise"), "{err}");
}
