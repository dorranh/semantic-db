use semantic_catalog::*;
#[test]
fn conflicts_preserve_origins_and_never_promote_proposals() {
    let fact = |id: &str, value, authority| Fact {
        id: id.into(),
        scope: "model/relation".into(),
        value,
        authority,
        origins: vec![SourceRef {
            artifact_revision: id.into(),
            path: "/rule".into(),
            span: None,
        }],
        evidence: vec![],
    };
    let authored = fact("a", false, Authority::Authored);
    let proposal = fact("b", true, Authority::Proposed);
    let FactResolution::Conflicting { alternatives } =
        resolve_facts([proposal.clone(), authored.clone()])
    else {
        panic!("conflict must survive")
    };
    assert_eq!(alternatives, vec![authored.clone(), proposal]);
    let FactResolution::Known {
        value,
        contributors,
    } = resolve_facts([authored.clone(), fact("c", false, Authority::Derived)])
    else {
        panic!("equivalent values retain contributors")
    };
    assert!(!value);
    assert_eq!(contributors.len(), 2);
    assert_eq!(
        resolve_facts(Vec::<Fact<bool>>::new()),
        FactResolution::Unknown
    );
}
#[test]
fn source_archive_debug_does_not_expose_bytes() {
    let archive = SourceArchive::new(b"private:source".as_slice(), "yaml", "1.2", "test");
    assert_eq!(archive.bytes(), b"private:source");
    assert!(!format!("{archive:?}").contains("private:source"));
    assert_eq!(archive.revision().len(), 64);
}
