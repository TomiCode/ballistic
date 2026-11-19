//! Unit tests on synthetic chunks laid out like the game's files.

use ckfile::{ClassId, Guid, StateChunk};

use crate::*;

fn chunk(class: ClassId, sections: &[(u32, Vec<u32>)], managers: Vec<u32>) -> StateChunk {
    let mut data = Vec::new();
    for (i, (id, payload)) in sections.iter().enumerate() {
        let next = if i + 1 == sections.len() {
            0
        } else {
            (data.len() + 2 + payload.len()) as u32
        };
        data.push(*id);
        data.push(next);
        data.extend_from_slice(payload);
    }
    StateChunk {
        class_id: class,
        data_version: 10,
        chunk_version: 7,
        options: ckfile::chunk_options::FILE,
        data,
        object_positions: vec![],
        sub_chunk_positions: vec![],
        manager_positions: managers,
    }
}

#[test]
fn building_block_layout() {
    // flags: building block + compatible class + target parameter
    let flags = behavior_flags::BUILDING_BLOCK
        | behavior_flags::COMPATIBLE_CLASS
        | behavior_flags::TARGET_PARAMETER;
    let lists = list_flags::INPUT_PARAMS
        | list_flags::LOCAL_PARAMS
        | list_flags::INPUTS
        | list_flags::OUTPUTS;
    let c = chunk(
        ClassId::BEHAVIOR,
        &[
            (0x10, vec![0]),
            (
                0x20,
                vec![
                    flags,
                    0x7522_370e,
                    0x37ec_15ec,
                    0x1_0000,
                    33,
                    71,
                    lists,
                    2,
                    43,
                    45,
                    1,
                    66,
                    2,
                    38,
                    39,
                    2,
                    40,
                    41,
                ],
            ),
        ],
        vec![],
    );
    let b = Behavior::decode(&c).unwrap();
    assert_eq!(
        b.prototype,
        Some(Prototype {
            guid: Guid(0x7522_370e, 0x37ec_15ec),
            version: 0x1_0000
        })
    );
    assert_eq!(b.compatible_class, Some(ClassId::ENTITY_3D));
    assert_eq!(b.target_parameter, Some(71));
    assert_eq!(b.input_params, vec![43, 45]);
    assert_eq!(b.local_params, vec![66]);
    assert_eq!(
        (b.inputs.clone(), b.outputs.clone()),
        (vec![38, 39], vec![40, 41])
    );
    assert!(!b.is_graph());
}

#[test]
fn graph_layout_with_operations() {
    let lists =
        list_flags::GRAPH | list_flags::OPERATIONS | list_flags::LOCAL_PARAMS | list_flags::INPUTS;
    let c = chunk(
        ClassId::BEHAVIOR,
        &[(
            0x20,
            vec![0, lists, 2, 10, 20, 1, 11, 1, 30, 2, 5, 6, 1, 12],
        )],
        vec![],
    );
    let b = Behavior::decode(&c).unwrap();
    assert!(b.is_graph());
    assert_eq!(b.sub_behaviors, vec![10, 20]);
    assert_eq!(b.links, vec![11]);
    assert_eq!(b.operations, vec![30]);
    assert_eq!(b.local_params, vec![5, 6]);
    assert_eq!(b.inputs, vec![12]);
}

#[test]
fn rejects_unknown_list_flags_and_trailing_words() {
    let c = chunk(ClassId::BEHAVIOR, &[(0x20, vec![0, 0x2])], vec![]);
    assert!(Behavior::decode(&c).is_err());
    let c = chunk(
        ClassId::BEHAVIOR,
        &[(0x20, vec![0, list_flags::INPUTS, 1, 5, 99])],
        vec![],
    );
    assert!(Behavior::decode(&c).is_err());
}

#[test]
fn parameter_value_kinds() {
    let float = Guid(0x4788_4c3f, 0x432c_2c20);
    let buf = chunk(
        ClassId::PARAMETER_LOCAL,
        &[(0x40, vec![float.0, float.1, 1, 4, 0.1f32.to_bits()])],
        vec![],
    );
    let l = ParameterLocal::decode(&buf).unwrap();
    assert_eq!(l.value, ParamValue::Buffer(0.1f32.to_le_bytes().to_vec()));

    let obj = chunk(
        ClassId::PARAMETER_LOCAL,
        &[(0x40, vec![1, 2, 2, 201])],
        vec![],
    );
    assert_eq!(
        ParameterLocal::decode(&obj).unwrap().value,
        ParamValue::Object(Some(201))
    );

    let none = chunk(ClassId::PARAMETER_OUT, &[(0x40, vec![1, 2, 3])], vec![]);
    assert_eq!(ParameterOut::decode(&none).unwrap().value, ParamValue::None);

    // Manager-scoped value: flagged by the manager position list (word 4).
    let msg = chunk(
        ClassId::PARAMETER_LOCAL,
        &[(0x40, vec![0x0388_1e12, 0x5ba3_4e2b, 0x466a_0fac, 0, 3])],
        vec![4],
    );
    assert_eq!(
        ParameterLocal::decode(&msg).unwrap().value,
        ParamValue::Manager {
            manager: Guid(0x466a_0fac, 0),
            value: 3
        }
    );

    // Custom serialisation: no size prefix (a GUID).
    let custom = chunk(
        ClassId::PARAMETER_LOCAL,
        &[(
            0x40,
            vec![0x3451_7df5, 0x045e_4965, 1, 0x1ad5_2a8e, 0x5e74_1920],
        )],
        vec![],
    );
    assert_eq!(
        ParameterLocal::decode(&custom).unwrap().value,
        ParamValue::Custom(vec![0x1ad5_2a8e, 0x5e74_1920])
    );

    // "This" parameter: no value section.
    let this = chunk(ClassId::PARAMETER_LOCAL, &[(0x200, vec![])], vec![]);
    let t = ParameterLocal::decode(&this).unwrap();
    assert!(t.is_myself);
}

#[test]
fn inputs_links_and_operations() {
    let pin = chunk(ClassId::PARAMETER_IN, &[(0x1000, vec![1, 2, 128])], vec![]);
    let p = ParameterIn::decode(&pin).unwrap();
    assert_eq!((p.source, p.shared, p.disabled), (Some(128), None, false));

    let shared = chunk(
        ClassId::PARAMETER_IN,
        &[(0x2000, vec![]), (0x800, vec![1, 2, 77])],
        vec![],
    );
    let p = ParameterIn::decode(&shared).unwrap();
    assert_eq!((p.source, p.shared, p.disabled), (None, Some(77), true));

    let link = chunk(
        ClassId::BEHAVIOR_LINK,
        &[(0x20, vec![0x1_0001, 40, 108])],
        vec![],
    );
    let l = BehaviorLink::decode(&link).unwrap();
    assert_eq!(
        (l.delay, l.initial_delay, l.source, l.target),
        (1, 1, Some(40), Some(108))
    );

    let op = chunk(
        ClassId::PARAMETER_OPERATION,
        &[(0x400, vec![0x1368_4596, 0x0ab5_7f34, 3, 450, 451, u32::MAX])],
        vec![],
    );
    let o = ParameterOperation::decode(&op).unwrap();
    assert_eq!(
        (o.input_a, o.input_b, o.output),
        (Some(450), Some(451), None)
    );
}
