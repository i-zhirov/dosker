use dosker_core::Status;

#[test]
fn every_allowed_transition_is_accepted() {
    let allowed = [
        (Status::Created, Status::Starting),
        (Status::Created, Status::Removed),
        (Status::Starting, Status::Running),
        (Status::Starting, Status::Exited),
        (Status::Starting, Status::Failed),
        (Status::Running, Status::Stopping),
        (Status::Running, Status::Exited),
        (Status::Stopping, Status::Stopped),
        (Status::Stopping, Status::Exited),
        (Status::Stopped, Status::Starting),
        (Status::Stopped, Status::Removed),
        (Status::Exited, Status::Starting),
        (Status::Exited, Status::Removed),
        (Status::Failed, Status::Removed),
    ];

    for (from, to) in allowed {
        assert!(
            from.can_transition_to(to),
            "{from} -> {to} should be accepted"
        );
    }
}

#[test]
fn forbidden_transitions_are_rejected() {
    let forbidden = [
        (Status::Created, Status::Running),
        (Status::Starting, Status::Stopped),
        (Status::Running, Status::Created),
        (Status::Stopped, Status::Running),
        (Status::Failed, Status::Starting),
        (Status::Removed, Status::Created),
    ];

    for (from, to) in forbidden {
        assert!(
            !from.can_transition_to(to),
            "{from} -> {to} should be rejected"
        );
    }
}

#[test]
fn active_states() {
    for status in [Status::Starting, Status::Running, Status::Stopping] {
        assert!(status.is_active(), "{status} should be active");
    }
    for status in [
        Status::Created,
        Status::Stopped,
        Status::Exited,
        Status::Failed,
        Status::Removed,
    ] {
        assert!(!status.is_active(), "{status} should not be active");
    }
}

#[test]
fn terminal_state() {
    assert!(Status::Removed.is_terminal());
    for status in [
        Status::Created,
        Status::Starting,
        Status::Running,
        Status::Stopping,
        Status::Stopped,
        Status::Exited,
        Status::Failed,
    ] {
        assert!(!status.is_terminal(), "{status} should not be terminal");
    }
}

#[test]
fn serializes_snake_case() {
    let cases = [
        (Status::Created, "created"),
        (Status::Starting, "starting"),
        (Status::Running, "running"),
        (Status::Stopping, "stopping"),
        (Status::Stopped, "stopped"),
        (Status::Exited, "exited"),
        (Status::Failed, "failed"),
        (Status::Removed, "removed"),
    ];

    for (status, expected) in cases {
        assert_eq!(
            serde_json::to_string(&status).unwrap(),
            format!("\"{expected}\"")
        );
        let back: Status = serde_json::from_str(&format!("\"{expected}\"")).unwrap();
        assert_eq!(back, status);
        assert_eq!(status.as_str(), expected);
        assert_eq!(status.to_string(), expected);
    }
}
