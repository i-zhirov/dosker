//! The control-transport contract every native backend must satisfy.

use std::io::{self, Read, Write};
use std::time::{Duration, Instant};

use dosker_core::ContainerId;
use dosker_platform::endpoint::control_endpoint;
use dosker_platform::{Endpoint, control};

fn endpoint() -> (tempfile::TempDir, Endpoint) {
    let dir = tempfile::tempdir().expect("create temp directory");
    let endpoint = control_endpoint(dir.path(), &ContainerId::generate());
    (dir, endpoint)
}

#[test]
fn round_trip_single_request() {
    let (_dir, endpoint) = endpoint();
    let listener = control::bind(&endpoint).expect("bind");
    let mut client = control::connect(&endpoint, Duration::from_secs(2)).expect("connect");
    let mut server = listener
        .accept(Duration::from_secs(2))
        .expect("accept")
        .expect("accepted connection");

    client.write_all(b"ping\n").expect("write request");
    client.flush().expect("flush request");

    let mut request = [0u8; 5];
    server
        .read_exact(&mut request)
        .expect("read the request line");
    assert_eq!(&request, b"ping\n");

    server.write_all(b"pong\n").expect("write response");
    server.flush().expect("flush response");

    let mut response = [0u8; 5];
    client
        .read_exact(&mut response)
        .expect("read the response line");
    assert_eq!(&response, b"pong\n");
}

#[test]
fn multiple_sequential_connections() {
    let (_dir, endpoint) = endpoint();
    let listener = control::bind(&endpoint).expect("bind");

    for _ in 0..3 {
        let mut client = control::connect(&endpoint, Duration::from_secs(2)).expect("connect");
        let mut server = listener
            .accept(Duration::from_secs(2))
            .expect("accept")
            .expect("accepted connection");

        client.write_all(b"ping\n").expect("write request");
        client.flush().expect("flush request");

        let mut request = [0u8; 5];
        server
            .read_exact(&mut request)
            .expect("read the request line");
        assert_eq!(&request, b"ping\n");

        server.write_all(b"pong\n").expect("write response");
        server.flush().expect("flush response");

        let mut response = [0u8; 5];
        client
            .read_exact(&mut response)
            .expect("read the response line");
        assert_eq!(&response, b"pong\n");
    }
}

#[test]
fn accept_returns_none_when_no_client_connects() {
    let (_dir, endpoint) = endpoint();
    let listener = control::bind(&endpoint).expect("bind");

    let started = Instant::now();
    let accepted = listener.accept(Duration::from_millis(100)).expect("accept");
    assert!(accepted.is_none());
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[test]
fn accept_returns_connection_after_client_connects() {
    let (_dir, endpoint) = endpoint();
    let listener = control::bind(&endpoint).expect("bind");
    let _client = control::connect(&endpoint, Duration::from_secs(2)).expect("connect");

    let accepted = listener.accept(Duration::from_secs(2)).expect("accept");
    assert!(accepted.is_some());
}

#[test]
fn server_read_times_out_on_idle_client() {
    let (_dir, endpoint) = endpoint();
    let listener = control::bind(&endpoint).expect("bind");
    let _client = control::connect(&endpoint, Duration::from_secs(2)).expect("connect");
    let mut server = listener
        .accept(Duration::from_secs(2))
        .expect("accept")
        .expect("accepted connection");

    server
        .set_read_timeout(Some(Duration::from_millis(100)))
        .expect("set read timeout");

    let mut byte = [0u8; 1];
    let error = server.read(&mut byte).expect_err("idle read must time out");
    assert!(matches!(
        error.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
    ));
}

#[test]
fn client_read_times_out_when_server_stays_silent() {
    let (_dir, endpoint) = endpoint();
    let listener = control::bind(&endpoint).expect("bind");
    let mut client = control::connect(&endpoint, Duration::from_secs(2)).expect("connect");
    let _server = listener
        .accept(Duration::from_secs(2))
        .expect("accept")
        .expect("accepted connection");

    client
        .set_read_timeout(Some(Duration::from_millis(100)))
        .expect("set read timeout");

    let mut byte = [0u8; 1];
    let error = client
        .read(&mut byte)
        .expect_err("silent read must time out");
    assert!(matches!(
        error.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
    ));
}

#[test]
fn connect_to_missing_endpoint_fails() {
    let (_dir, endpoint) = endpoint();

    let started = Instant::now();
    let result = control::connect(&endpoint, Duration::from_millis(200));
    assert!(result.is_err());
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[test]
fn endpoint_alive_is_false_for_missing_endpoint() {
    let (_dir, endpoint) = endpoint();
    assert!(!control::endpoint_alive(&endpoint));
}

#[test]
fn endpoint_alive_is_true_for_bound_endpoint() {
    let (_dir, endpoint) = endpoint();
    let _listener = control::bind(&endpoint).expect("bind");
    assert!(control::endpoint_alive(&endpoint));
}

#[test]
fn large_payload_round_trip() {
    let (_dir, endpoint) = endpoint();
    let listener = control::bind(&endpoint).expect("bind");
    let mut client = control::connect(&endpoint, Duration::from_secs(2)).expect("connect");
    let mut server = listener
        .accept(Duration::from_secs(2))
        .expect("accept")
        .expect("accepted connection");

    let payload: Vec<u8> = (0..1024 * 1024).map(|index| (index % 251) as u8).collect();
    let written = payload.clone();

    let writer = std::thread::spawn(move || {
        client.write_all(&written).expect("write payload");
        client.flush().expect("flush payload");
    });

    let mut received = vec![0u8; payload.len()];
    server
        .read_exact(&mut received)
        .expect("read the payload back");
    writer.join().expect("writer thread");

    assert_eq!(received, payload);
}

#[cfg(unix)]
#[test]
fn socket_and_directory_permissions_are_owner_only() {
    use std::os::unix::fs::PermissionsExt;

    let (_dir, endpoint) = endpoint();
    let _listener = control::bind(&endpoint).expect("bind");

    let path = match &endpoint {
        Endpoint::UnixSocket(path) => path.clone(),
        Endpoint::NamedPipe(_) => panic!("expected a unix socket endpoint"),
    };
    let parent = path.parent().expect("socket parent directory");

    let directory_mode = std::fs::metadata(parent)
        .expect("socket directory metadata")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(directory_mode, 0o700);

    let socket_mode = std::fs::metadata(&path)
        .expect("socket file metadata")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(socket_mode, 0o600);
}

#[cfg(unix)]
#[test]
fn bind_fails_when_the_endpoint_is_live() {
    let (_dir, endpoint) = endpoint();
    let _first = control::bind(&endpoint).expect("first bind");

    let error = match control::bind(&endpoint) {
        Ok(_) => panic!("second bind must fail"),
        Err(error) => error,
    };
    match error {
        dosker_platform::PlatformError::Io { source, .. } => {
            assert_eq!(source.kind(), io::ErrorKind::AddrInUse);
        }
        other => panic!("unexpected bind error: {other:?}"),
    }
}

#[cfg(unix)]
#[test]
fn long_root_uses_the_short_fallback() {
    let dir = tempfile::tempdir().expect("create temp directory");
    let root = dir.path().join("r".repeat(200));
    std::fs::create_dir_all(&root).expect("create long root");
    let id = ContainerId::generate();
    let endpoint = control_endpoint(&root, &id);

    assert_eq!(
        endpoint,
        Endpoint::UnixSocket(dosker_platform::short_socket_path(&id))
    );

    let listener = control::bind(&endpoint).expect("bind");
    let mut client = control::connect(&endpoint, Duration::from_secs(2)).expect("connect");
    let mut server = listener
        .accept(Duration::from_secs(2))
        .expect("accept")
        .expect("accepted connection");

    client.write_all(b"ping\n").expect("write request");
    client.flush().expect("flush request");

    let mut request = [0u8; 5];
    server
        .read_exact(&mut request)
        .expect("read the request line");
    assert_eq!(&request, b"ping\n");

    server.write_all(b"pong\n").expect("write response");
    server.flush().expect("flush response");

    let mut response = [0u8; 5];
    client
        .read_exact(&mut response)
        .expect("read the response line");
    assert_eq!(&response, b"pong\n");

    control::remove_endpoint(&endpoint).expect("remove endpoint");
}

#[cfg(windows)]
#[test]
fn pipe_is_connectable_by_current_user() {
    let (_dir, endpoint) = endpoint();
    let listener = control::bind(&endpoint).expect("bind");
    let mut client = control::connect(&endpoint, Duration::from_secs(2)).expect("connect");
    let mut server = listener
        .accept(Duration::from_secs(2))
        .expect("accept")
        .expect("accepted connection");

    client.write_all(b"ping\n").expect("write request");
    client.flush().expect("flush request");

    let mut request = [0u8; 5];
    server
        .read_exact(&mut request)
        .expect("read the request line");
    assert_eq!(&request, b"ping\n");

    server.write_all(b"pong\n").expect("write response");
    server.flush().expect("flush response");

    let mut response = [0u8; 5];
    client
        .read_exact(&mut response)
        .expect("read the response line");
    assert_eq!(&response, b"pong\n");
}

#[cfg(windows)]
#[test]
fn pipe_acl_has_one_ace_for_current_user() {
    let (_dir, endpoint) = endpoint();
    let _listener = control::bind(&endpoint).expect("bind");

    let name = match &endpoint {
        Endpoint::NamedPipe(name) => name.clone(),
        Endpoint::UnixSocket(_) => panic!("expected a named pipe endpoint"),
    };

    assert_pipe_acl_names_current_user(&name);
}

#[cfg(windows)]
fn assert_pipe_acl_names_current_user(name: &str) {
    use std::ffi::c_void;
    use std::mem::{size_of, zeroed};
    use std::ptr::{null, null_mut};

    use windows_sys::Win32::Foundation::{
        CloseHandle, ERROR_SUCCESS, GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE, LocalFree,
    };
    use windows_sys::Win32::Security::Authorization::{GetSecurityInfo, SE_KERNEL_OBJECT};
    use windows_sys::Win32::Security::{
        ACCESS_ALLOWED_ACE, ACL, ACL_SIZE_INFORMATION, AclSizeInformation,
        DACL_SECURITY_INFORMATION, EqualSid, GetAce, GetAclInformation, PSECURITY_DESCRIPTOR, PSID,
    };
    use windows_sys::Win32::Storage::FileSystem::{CreateFileW, OPEN_EXISTING, READ_CONTROL};
    use windows_sys::Win32::System::SystemServices::ACCESS_ALLOWED_ACE_TYPE;

    let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            GENERIC_READ | GENERIC_WRITE | READ_CONTROL,
            0,
            null(),
            OPEN_EXISTING,
            0,
            null_mut(),
        )
    };
    assert_ne!(handle, INVALID_HANDLE_VALUE, "open the named pipe");

    let mut dacl: *mut ACL = null_mut();
    let mut descriptor: PSECURITY_DESCRIPTOR = null_mut();
    let status = unsafe {
        GetSecurityInfo(
            handle,
            SE_KERNEL_OBJECT,
            DACL_SECURITY_INFORMATION,
            null_mut(),
            null_mut(),
            &mut dacl,
            null_mut(),
            &mut descriptor,
        )
    };
    assert_eq!(status, ERROR_SUCCESS, "query the pipe security descriptor");
    assert!(!dacl.is_null(), "the pipe has a DACL");

    let mut size_info: ACL_SIZE_INFORMATION = unsafe { zeroed() };
    let read_size = unsafe {
        GetAclInformation(
            dacl,
            &mut size_info as *mut _ as *mut c_void,
            size_of::<ACL_SIZE_INFORMATION>() as u32,
            AclSizeInformation,
        )
    };
    assert_ne!(read_size, 0, "read the ACL size information");
    assert_eq!(size_info.AceCount, 1, "the DACL has one entry");

    let mut ace: *mut c_void = null_mut();
    let read_ace = unsafe { GetAce(dacl, 0, &mut ace) };
    assert_ne!(read_ace, 0, "read the access control entry");
    let ace = unsafe { &*(ace as *const ACCESS_ALLOWED_ACE) };
    assert_eq!(
        u32::from(ace.Header.AceType),
        ACCESS_ALLOWED_ACE_TYPE,
        "the entry allows access"
    );

    let ace_sid = &ace.SidStart as *const u32 as PSID;
    let current_sid = current_user_sid();
    let equals = unsafe { EqualSid(ace_sid, current_sid.as_ptr() as PSID) };
    assert_ne!(equals, 0, "the entry names the current user");

    unsafe {
        LocalFree(descriptor);
        CloseHandle(handle);
    }
}

#[cfg(windows)]
fn current_user_sid() -> Vec<u8> {
    use std::mem::size_of;
    use std::ptr::null_mut;

    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::Security::{
        GetLengthSid, GetTokenInformation, TOKEN_QUERY, TOKEN_USER, TokenUser,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    unsafe {
        let mut token: HANDLE = null_mut();
        let opened = OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token);
        assert_ne!(opened, 0, "open the current process token");

        let mut needed = 0u32;
        GetTokenInformation(token, TokenUser, null_mut(), 0, &mut needed);
        assert!(needed > 0, "read the token user size");

        let words = (needed as usize).div_ceil(size_of::<usize>());
        let mut buffer = vec![0usize; words];
        let filled = GetTokenInformation(
            token,
            TokenUser,
            buffer.as_mut_ptr() as *mut _,
            needed,
            &mut needed,
        );
        assert_ne!(filled, 0, "read the token user");

        let user = &*(buffer.as_ptr() as *const TOKEN_USER);
        let length = GetLengthSid(user.User.Sid) as usize;
        let sid = std::slice::from_raw_parts(user.User.Sid as *const u8, length).to_vec();

        CloseHandle(token);
        sid
    }
}
