use crate::pktline::*;

#[test]
fn pkt_line_encodes_length_prefix() {
    // "# service=git-upload-pack\n" is 26 bytes + 4 = 30 = 0x1e.
    let line = pkt_line("# service=git-upload-pack\n");
    assert_eq!(&line[..4], b"001e");
    assert_eq!(&line[4..], b"# service=git-upload-pack\n");
}

#[test]
fn service_banner_ends_with_flush() {
    let banner = service_banner("git-upload-pack");
    assert!(banner.ends_with(b"0000"));
    assert!(banner.starts_with(b"001e# service=git-upload-pack\n"));
}
