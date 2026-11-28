use openbooks_core::{Email, Refusal, Role, Word};

#[test]
fn every_role_against_every_word_holds_the_table() {
    let table = [
        (Role::Owner, [true, true, true, true, true]),
        (Role::Accountant, [true, true, true, true, false]),
        (Role::Staff, [true, false, true, false, false]),
        (Role::Viewer, [true, true, false, false, false]),
    ];
    assert_eq!(table.map(|(role, _)| role), Role::ALL);
    for (role, row) in table {
        assert_eq!(Word::ALL.map(|word| role.allows(word)), row, "{role:?}");
    }
}

#[test]
fn a_role_reads_back_from_its_name_and_an_email_is_kept_in_lower_case() -> Result<(), Refusal> {
    for role in Role::ALL {
        assert_eq!(Role::parse(role.as_str())?, role);
    }
    assert_eq!(Role::parse("admin"), Err(Refusal::BadRole));
    assert_eq!(Email::new(" Ann@Example.COM ")?.as_str(), "ann@example.com");
    for bad in ["", "ann", "@example.com", "ann@example", "ann@b@example.com"] {
        assert_eq!(Email::new(bad), Err(Refusal::BadEmail), "{bad}");
    }
    Ok(())
}
