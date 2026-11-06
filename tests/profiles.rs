//! Profile discovery: at least one shell is found on this machine.

#[test]
fn discovers_a_shell() {
    let profiles = nsh_app_myterm::profiles::discover();
    assert!(!profiles.is_empty(), "no shell profile found");
    for profile in &profiles {
        assert!(!profile.name.is_empty());
        assert!(!profile.program.is_empty());
    }
    assert!(
        profiles.iter().any(|profile| profile.program.to_ascii_lowercase().ends_with(".exe")),
        "a profile must point at an executable"
    );
}
