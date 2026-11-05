use super::*;

fn parsed(arguments: &[&str]) -> Result<Cli, clap::Error> {
    parse(arguments.iter().map(OsString::from))
}

#[test]
fn the_observer_names_the_two_tests_as_brk_does() {
    let observed = ["--server-pid", "1", "--output", "run.jsonl", "--phase", "1s", "--timeout", "1s"];
    let url = "http://example.test/plain";
    let mut requests = vec!["brk-observe", url, "--conn", "4", "--pace", "1"];
    requests.extend(observed);
    assert!(matches!(parsed(&requests).unwrap().test, Test::Requests(_)));
    for name in ["connections", "conn"] {
        let mut connections = vec!["brk-observe", name, url, "--pace", "8", "--requests", "2"];
        connections.extend(observed);
        let Test::Connections(test) = parsed(&connections).unwrap().test else {
            panic!("{name} selects the connection test");
        };
        assert_eq!((test.start, test.pace, test.requests), (None, 8, 2));
        connections.extend(["--conn", "4"]);
        assert!(parsed(&connections).is_err(), "{name}");
    }
}
