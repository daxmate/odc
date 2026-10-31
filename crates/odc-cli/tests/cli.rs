use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_dir(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    let dir = std::env::temp_dir().join(format!("odc-cli-{label}-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn odc(repo: &PathBuf, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_odc"))
        .arg("--repo")
        .arg(repo)
        .args(args)
        .output()
        .expect("run odc")
}

fn stdout(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn cli_full_cycle() {
    let dir = temp_dir("cycle");
    let repo = dir.join("repo");
    let data = dir.join("words.jsonl");
    std::fs::write(
        &data,
        concat!(
            "{\"word\":\"alpha\",\"defs\":[{\"pos\":\"noun\",\"sense\":\"first\"}]}\n",
            "{\"word\":\"beta\",\"defs\":[{\"pos\":\"noun\",\"sense\":\"second\"}]}\n",
            "{\"word\":\"gamma\",\"defs\":[{\"pos\":\"verb\",\"sense\":\"third\"}]}\n"
        ),
    )
    .expect("write data");

    let init = odc(&repo, &["init"]);
    assert!(init.status.success(), "{}", stderr(&init));
    assert!(stdout(&init).contains("genesis"));

    let import = odc(
        &repo,
        &["import", data.to_str().expect("path"), "--format", "jsonl"],
    );
    assert!(import.status.success(), "{}", stderr(&import));
    assert!(stdout(&import).contains("working set = 3"));

    let commit = odc(&repo, &["commit", "-m", "first import", "--timestamp", "1"]);
    assert!(commit.status.success(), "{}", stderr(&commit));

    let status = odc(&repo, &["status"]);
    assert!(stdout(&status).contains("working set clean (3 entries)"));

    let show = odc(&repo, &["show", "Alpha", "--json"]);
    assert!(show.status.success(), "{}", stderr(&show));
    assert!(stdout(&show).contains("\"alpha\""));

    let log = odc(&repo, &["log"]);
    assert!(stdout(&log).contains("first import"));

    let diff = odc(&repo, &["diff", "HEAD~1", "HEAD", "--json"]);
    assert!(diff.status.success(), "{}", stderr(&diff));
    assert!(stdout(&diff).contains("alpha"));

    let verify = odc(&repo, &["verify"]);
    assert!(verify.status.success(), "{}", stderr(&verify));
    assert!(stdout(&verify).contains("verify: OK"));

    let patch_path = dir.join("cycle.odp");
    let patch = odc(
        &repo,
        &[
            "patch",
            "create",
            "HEAD~1",
            "HEAD",
            "-o",
            patch_path.to_str().expect("patch path"),
        ],
    );
    assert!(patch.status.success(), "{}", stderr(&patch));
    assert!(stdout(&patch).contains("wrote"));
    assert!(patch_path.exists());

    let _ = std::fs::remove_dir_all(&dir);
}
