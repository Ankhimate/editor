//! The layered-document read surface, driven from JavaScript.
//!
//! The tag grammar and inference are the reusable half of the PSD importer:
//! neither is about Photoshop. Without them a plugin importing a layered TIFF
//! or a directory of numbered PNGs reimplements both, so `[bones]` would mean
//! one thing in the built-in importer and another in an addon — which is the
//! failure a shared vocabulary exists to prevent.
//!
//! These run the real fixture through the real host, because that is the only
//! check that has found anything: a binding that compiles proves nothing about
//! whether the JSON reaches the script in a shape it can read.

use ankhimate_plugins::Host;

const FIXTURE: &[u8] = include_bytes!("../../formats/tests/fixtures/tagged_rig.psd");

/// Run `script` with the fixture available as `PSD_BASE64`, returning whatever
/// it logs.
fn run(script: &str) -> Vec<String> {
    let base64 = ankhimate_plugins::importer::encode_base64(FIXTURE);
    let host = Host::new();
    let mut edit = ankhimate_document::Edit::default();
    let full = format!("const PSD_BASE64 = \"{base64}\";\n{script}");
    host.run(&full, &mut edit).expect("the script runs")
}

#[test]
fn a_plugin_can_read_a_psds_structure() {
    let log = run(r#"
        const psd = ankhimate.readPsd(PSD_BASE64);
        console.log(String(psd.layers.length));
        const arm = psd.layers.find(l => l.name === "arm");
        console.log(arm.name);
        console.log(arm.raw_name);
        console.log(arm.path);
        console.log(JSON.stringify(Object.keys(arm.tags)));
        console.log(String(arm.is_group));
        "#);

    assert_eq!(log[0], "16", "every layer and group reached the script");
    assert_eq!(
        log[1], "arm",
        "the name arrives with its tags stripped — a plugin should not have to \
         know the grammar to find out what a bone is called"
    );
    assert_eq!(
        log[2], "arm [bone]",
        "and the raw name is there when wanted"
    );
    // **The path is built from raw names**, so it carries the tags too. That is
    // a wart rather than a decision: a re-import matches on the path, so adding
    // a tag to a group renames every path beneath it and the match is lost. It
    // is pinned here rather than quietly fixed because `psd_layer_paths` is
    // already saved in that shape and changing it needs a migration.
    assert_eq!(log[3], "arm [bone]");
    assert_eq!(log[4], r#"["bone"]"#);
    assert_eq!(log[5], "true");
}

#[test]
fn inference_reaches_the_script_with_its_reasons() {
    // The half that makes inference safe rather than merely convenient. A
    // plugin that can read the guesses can show them; one that cannot has to
    // decide silently, which is the thing this whole surface exists to avoid.
    let log = run(r#"
        const psd = ankhimate.readPsd(PSD_BASE64);
        const sequence = psd.layers.map(l => l.sequence).find(Boolean);
        console.log(sequence.stem);
        console.log(JSON.stringify(sequence.frames));
        const face = psd.guesses.find(g => g.path === "face");
        console.log(face.decided);
        console.log(String(face.override_with.length > 0));
        "#);

    assert_eq!(log[0], "fire");
    assert_eq!(log[1], r#"["fx/fire_01","fx/fire_02","fx/fire_03"]"#);
    assert!(log[2].contains("one bone"), "{}", log[2]);
    assert_eq!(
        log[3], "true",
        "and every guess says how to disagree with it"
    );
}

#[test]
fn the_tag_grammar_works_on_a_name_from_any_format() {
    // The point of exposing this separately: an Aseprite or TIFF importer wants
    // `[bone]` to mean what it means here, not what its author guessed.
    let log = run(r#"
        const parsed = ankhimate.parseTags("cape [bone][physics:cloth][wobble]");
        console.log(parsed.name);
        console.log(parsed.tags.physics);
        console.log(String("bone" in parsed.tags));
        console.log(JSON.stringify(parsed.unknown_tags));
        "#);

    assert_eq!(log[0], "cape");
    assert_eq!(log[1], "cloth");
    assert_eq!(log[2], "true");
    assert_eq!(
        log[3], r#"["wobble"]"#,
        "an unrecognised tag is handed over rather than dropped — a plugin is \
         the one consumer that can define new ones"
    );
}

#[test]
fn a_plugin_can_infer_over_layers_it_built_itself() {
    // The other reusable half. "Is this a flipbook" is a question about a layer
    // tree, not about Photoshop, and a plugin reading a directory of numbered
    // PNGs should not have to answer it again — differently.
    let log = run(r#"
        const layer = (path, name) => ({
          path, name, raw_name: name, depth: 0, is_group: false, visible: true,
          bounds: [0, 0, 10, 10], tags: {}, unknown_tags: [],
          bone: true, sequence: null, mirrors: null,
        });
        const result = ankhimate.infer([
          layer("boom_01", "boom_01"),
          layer("boom_02", "boom_02"),
          layer("boom_03", "boom_03"),
        ]);
        const sequence = result.layers.map(l => l.sequence).find(Boolean);
        console.log(sequence.stem);
        console.log(String(sequence.frames.length));
        console.log(String(result.guesses.length > 0));
        "#);

    assert_eq!(log[0], "boom");
    assert_eq!(log[1], "3");
    assert_eq!(
        log[2], "true",
        "and it is reported, so the plugin can show what it decided"
    );
}

#[test]
fn a_bad_psd_throws_rather_than_returning_nothing() {
    // Returning null would let a plugin carry on and build a rig out of
    // nothing. The stack trace naming the line is the whole difference between
    // a plugin author finding this in a second and finding it in an hour.
    let host = Host::new();
    let mut edit = ankhimate_document::Edit::default();
    let error = host
        .run(r#"ankhimate.readPsd("bm90IGEgcHNk");"#, &mut edit)
        .expect_err("a PSD that is not one is an error");
    let message = error.to_string();
    assert!(
        !message.contains("Exception generated by QuickJS"),
        "the reason has to survive the scope it was thrown in: {message}"
    );
}
