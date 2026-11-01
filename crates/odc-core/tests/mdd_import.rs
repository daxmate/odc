//! MDD resource import: volume discovery, ordering, key normalisation and
//! the warnings a broken volume produces.

use odc_core::mdd_import::{discover_volumes, normalize_resource_key};
use odc_core::pack::PackOptions;
use odc_core::{import_mdx, MdxImportOptions, Repo};
use odc_testkit::{write_mdd, write_v2_mdx};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// Bytes that travel through the importer unchanged.
const IMAGE: &[u8] = b"\x89PNG\r\n\x1a\n pretend png";
/// A second payload used to tell volumes apart.
const OTHER: &[u8] = b"ID3\x03 pretend mp3";

struct Fixture {
    root: PathBuf,
    mdx: PathBuf,
    repo: PathBuf,
}

impl Fixture {
    fn new(label: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("odc-mdd-{label}-{nanos}"));
        std::fs::create_dir_all(&root).expect("temp dir");
        let mdx = root.join("dictionary.mdx");
        write_v2_mdx(&mdx, &[("apple", "<img src=\"pic/apple.png\">")]).expect("mdx fixture");
        Self {
            root: root.clone(),
            mdx,
            repo: root.join("repo"),
        }
    }

    /// Path of a volume file, e.g. `dictionary.2.mdd`.
    fn volume(&self, suffix: &str) -> PathBuf {
        if suffix.is_empty() {
            self.root.join("dictionary.mdd")
        } else {
            self.root.join(format!("dictionary.{suffix}.mdd"))
        }
    }

    fn import(&self, options: MdxImportOptions) -> odc_core::MdxImportReport {
        let mut repo = Repo::init(&self.repo).expect("init");
        import_mdx(&self.mdx, &mut repo, options).expect("import")
    }

    /// Import into the repository the fixture already created.
    fn import_again(&self, options: MdxImportOptions) -> odc_core::MdxImportReport {
        let mut repo = self.open();
        import_mdx(&self.mdx, &mut repo, options).expect("import")
    }

    fn open(&self) -> Repo {
        Repo::open(&self.repo).expect("open")
    }

    /// Open the repository read-only for a pack run.
    fn open_for_pack(&self) -> Repo {
        Repo::open(&self.repo).expect("open")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn imports_a_single_volume_verbatim() {
    let fixture = Fixture::new("single");
    write_mdd(&fixture.volume(""), &[("\\pic\\apple.png", IMAGE)]).expect("mdd");

    let report = fixture.import(MdxImportOptions::default());

    assert_eq!(report.mdd_files.len(), 1);
    assert_eq!(report.mdd_files[0].entries, 1);
    assert_eq!(report.mdd_files[0].bytes, IMAGE.len() as u64);
    assert_eq!(report.resource_count, 1);
    assert_eq!(report.resource_bytes, IMAGE.len() as u64);
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);

    let repo = fixture.open();
    assert_eq!(repo.resource_count().expect("count"), 1);
    assert_eq!(
        repo.resource("pic/apple.png").expect("lookup"),
        Some(IMAGE.to_vec())
    );
}

#[test]
fn normalises_resource_keys() {
    assert_eq!(normalize_resource_key("\\pic\\apple.png"), "pic/apple.png");
    assert_eq!(normalize_resource_key("/pic/apple.png"), "pic/apple.png");
    assert_eq!(normalize_resource_key("./pic/apple.png"), "pic/apple.png");
    assert_eq!(normalize_resource_key("pic/apple.png"), "pic/apple.png");
    assert_eq!(normalize_resource_key(""), "");
}

#[test]
fn lookup_ignores_case_and_leading_backslashes() {
    let fixture = Fixture::new("case");
    write_mdd(&fixture.volume(""), &[("\\pic\\Apple.png", IMAGE)]).expect("mdd");
    fixture.import(MdxImportOptions::default());

    let repo = fixture.open();
    for key in [
        "pic/Apple.png",
        "pic/apple.png",
        "\\pic\\apple.png",
        "/PIC/APPLE.PNG",
    ] {
        assert_eq!(
            repo.resource(key).expect("lookup"),
            Some(IMAGE.to_vec()),
            "lookup of {key}"
        );
    }
    assert_eq!(repo.resource("pic/missing.png").expect("lookup"), None);
}

#[test]
fn main_volume_wins_over_numbered_ones() {
    let fixture = Fixture::new("main-wins");
    write_mdd(&fixture.volume(""), &[("\\pic\\apple.png", IMAGE)]).expect("main");
    write_mdd(&fixture.volume("1"), &[("\\pic\\apple.png", OTHER)]).expect("volume 1");
    write_mdd(&fixture.volume("2"), &[("\\sound\\apple.mp3", OTHER)]).expect("volume 2");

    let report = fixture.import(MdxImportOptions::default());

    assert_eq!(report.mdd_files.len(), 3);
    assert_eq!(report.resource_count, 2);
    let repo = fixture.open();
    assert_eq!(
        repo.resource("pic/apple.png").expect("lookup"),
        Some(IMAGE.to_vec()),
        "the main volume is read first and keeps its payload"
    );
    assert_eq!(
        repo.resource("sound/apple.mp3").expect("lookup"),
        Some(OTHER.to_vec())
    );
}

#[test]
fn numbered_volumes_sort_numerically() {
    let fixture = Fixture::new("order");
    write_mdd(&fixture.volume("10"), &[("\\pic\\ten.png", OTHER)]).expect("volume 10");
    write_mdd(&fixture.volume("2"), &[("\\pic\\two.png", IMAGE)]).expect("volume 2");
    write_mdd(&fixture.volume(""), &[("\\pic\\main.png", IMAGE)]).expect("main");

    let volumes = discover_volumes(&fixture.mdx);
    let names: Vec<String> = volumes
        .iter()
        .map(|path| {
            path.file_name()
                .expect("name")
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    assert_eq!(
        names,
        vec![
            "dictionary.mdd".to_string(),
            "dictionary.2.mdd".to_string(),
            "dictionary.10.mdd".to_string()
        ]
    );
}

#[test]
fn duplicate_key_between_numbered_volumes_keeps_the_lower_number() {
    let fixture = Fixture::new("duplicate");
    write_mdd(&fixture.volume("2"), &[("\\pic\\apple.png", IMAGE)]).expect("volume 2");
    write_mdd(&fixture.volume("10"), &[("\\pic\\apple.png", OTHER)]).expect("volume 10");

    let report = fixture.import(MdxImportOptions::default());

    assert_eq!(report.resource_count, 1);
    let repo = fixture.open();
    assert_eq!(
        repo.resource("pic/apple.png").expect("lookup"),
        Some(IMAGE.to_vec())
    );
}

#[test]
fn missing_volumes_are_a_no_op() {
    let fixture = Fixture::new("no-mdd");
    let report = fixture.import(MdxImportOptions::default());

    assert_eq!(report.imported_entries, 1);
    assert!(report.mdd_files.is_empty());
    assert_eq!(report.resource_count, 0);
    assert_eq!(report.resource_bytes, 0);
    assert!(report.warnings.is_empty());
    assert_eq!(fixture.open().resource_count().expect("count"), 0);
}

#[test]
fn corrupt_volume_warns_but_keeps_the_import() {
    let fixture = Fixture::new("corrupt");
    write_mdd(&fixture.volume(""), &[("\\pic\\apple.png", IMAGE)]).expect("main");
    std::fs::write(fixture.volume("1"), b"not an mdd file at all").expect("corrupt volume");

    let report = fixture.import(MdxImportOptions::default());

    assert_eq!(report.imported_entries, 1, "the MDX import still succeeds");
    assert_eq!(report.mdd_files.len(), 1);
    assert_eq!(report.warnings.len(), 1);
    assert!(
        report.warnings[0].contains("dictionary.1.mdd"),
        "{}",
        report.warnings[0]
    );
    assert_eq!(report.resource_count, 1);
}

#[test]
fn explicit_volume_list_replaces_discovery() {
    let fixture = Fixture::new("explicit");
    let elsewhere = fixture.root.join("elsewhere.mdd");
    write_mdd(&elsewhere, &[("\\pic\\only.png", OTHER)]).expect("explicit volume");
    // A discovered volume that must be ignored because the caller named one.
    write_mdd(&fixture.volume(""), &[("\\pic\\ignored.png", IMAGE)]).expect("main");

    let report = fixture.import(MdxImportOptions {
        mdd_volumes: Some(vec![elsewhere.clone()]),
        ..MdxImportOptions::default()
    });

    assert_eq!(report.mdd_files.len(), 1);
    assert_eq!(report.mdd_files[0].path, elsewhere);
    let repo = fixture.open();
    assert_eq!(
        repo.resource("pic/only.png").expect("lookup"),
        Some(OTHER.to_vec())
    );
    assert_eq!(repo.resource("pic/ignored.png").expect("lookup"), None);
}

#[test]
fn import_mdd_can_be_switched_off() {
    let fixture = Fixture::new("off");
    write_mdd(&fixture.volume(""), &[("\\pic\\apple.png", IMAGE)]).expect("main");

    let report = fixture.import(MdxImportOptions {
        import_mdd: false,
        ..MdxImportOptions::default()
    });

    assert!(report.mdd_files.is_empty());
    assert_eq!(report.resource_count, 0);
    assert_eq!(fixture.open().resource_count().expect("count"), 0);
}

#[test]
fn resources_survive_a_pack() {
    let fixture = Fixture::new("pack");
    write_mdd(&fixture.volume(""), &[("\\pic\\apple.png", IMAGE)]).expect("main");
    fixture.import(MdxImportOptions::default());
    let mut repo = fixture.open();
    repo.commit("import", "tester", 1_700_000_000)
        .expect("commit");

    for shallow in [false, true] {
        let out = fixture.root.join(format!("packed-shallow-{shallow}.odc"));
        let report = repo
            .pack(
                &out,
                PackOptions {
                    shallow,
                    force: false,
                },
            )
            .expect("pack");
        assert!(report.blocks >= 1);

        let packed = Repo::open(&out).expect("open pack");
        assert_eq!(
            packed.resource_count().expect("count"),
            1,
            "shallow={shallow}"
        );
        assert_eq!(
            packed.resource("pic/apple.png").expect("lookup"),
            Some(IMAGE.to_vec()),
            "shallow={shallow}"
        );
    }
}

#[test]
fn old_database_gains_the_resources_table_on_open() {
    let fixture = Fixture::new("old-db");
    fixture.import(MdxImportOptions::default());
    let db = fixture.repo.join("odc.db");
    {
        let connection = rusqlite::Connection::open(&db).expect("open db");
        connection
            .execute("DROP TABLE resources", [])
            .expect("drop table");
    }

    let repo = Repo::open(&fixture.repo).expect("reopen old database");
    assert_eq!(repo.resource_count().expect("count"), 0);
    assert_eq!(repo.resource("pic/apple.png").expect("lookup"), None);
}

#[test]
fn packing_a_database_without_the_resources_table_works() {
    let fixture = Fixture::new("old-pack");
    fixture.import(MdxImportOptions::default());
    let mut repo = fixture.open();
    repo.commit("import", "tester", 1_700_000_000)
        .expect("commit");
    drop(repo);
    let db = fixture.repo.join("odc.db");
    {
        let connection = rusqlite::Connection::open(&db).expect("open db");
        connection
            .execute("DROP TABLE resources", [])
            .expect("drop table");
    }
    let repo = fixture.open_for_pack();

    let out = fixture.root.join("packed-old.odc");
    repo.pack(
        &out,
        PackOptions {
            shallow: true,
            force: false,
        },
    )
    .expect("pack");
    let packed = Repo::open(&out).expect("open pack");
    assert_eq!(packed.resource_count().expect("count"), 0);
}

#[test]
fn reimport_replaces_the_payload_for_a_key() {
    let fixture = Fixture::new("replace");
    write_mdd(&fixture.volume(""), &[("\\pic\\apple.png", IMAGE)]).expect("first");
    fixture.import(MdxImportOptions::default());

    write_mdd(&fixture.volume(""), &[("\\pic\\apple.png", OTHER)]).expect("second");
    let report = fixture.import_again(MdxImportOptions::default());

    assert_eq!(report.resource_count, 1);
    let repo = fixture.open();
    assert_eq!(
        repo.resource("pic/apple.png").expect("lookup"),
        Some(OTHER.to_vec())
    );
    assert_eq!(
        repo.resource_bytes().expect("bytes"),
        OTHER.len() as u64,
        "the old payload is no longer indexed"
    );
}
