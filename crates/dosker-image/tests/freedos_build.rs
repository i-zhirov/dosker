mod support;

use dosker_image::freedos::{
    BASE_AUTOEXEC_BYTES, BOOT_SECTOR_TEMPLATE, BaseImage, CONFIG_SYS_BYTES, DISK_SIZE,
    FreedosBinaries, FreedosVersion, MBR_TEMPLATE, build_base_image,
};
use dosker_image::{
    Attributes, BootLayout, DosPath, FatType, Geometry, ImageError, TreeNode, fixed_timestamp,
};

const KERNEL: &[u8] = b"synthetic kernel bytes";
const COMMAND: &[u8] = b"synthetic command bytes";
const AGENT: &[u8] = b"synthetic agent bytes";
const PARTITION_OFFSET: usize = 2048 * 512;

fn binaries() -> FreedosBinaries {
    FreedosBinaries {
        kernel: KERNEL.to_vec(),
        command: COMMAND.to_vec(),
        agent: AGENT.to_vec(),
        mbr: MBR_TEMPLATE.to_vec(),
        boot_sector: BOOT_SECTOR_TEMPLATE.to_vec(),
    }
}

fn version() -> FreedosVersion {
    FreedosVersion::parse("1.3").unwrap()
}

fn file_bytes<'a>(image: &'a BaseImage, path: &str) -> &'a [u8] {
    match image.tree.get(&DosPath::parse(path).unwrap()).unwrap() {
        TreeNode::File(file) => &file.bytes,
        other => panic!("expected a file at {path}, got {other:?}"),
    }
}

#[test]
fn build_is_repeatable() {
    let first = build_base_image(&binaries(), &version()).unwrap();
    let second = build_base_image(&binaries(), &version()).unwrap();

    assert_eq!(first.raw, second.raw);
    assert_eq!(first.tree.digest().unwrap(), second.tree.digest().unwrap());
    assert_eq!(first.boot, second.boot);
}

#[test]
fn build_has_the_pinned_geometry() {
    let image = build_base_image(&binaries(), &version()).unwrap();

    assert_eq!(image.raw.len() as u64, 33_554_432);
    assert_eq!(DISK_SIZE, 33_554_432);

    let entry = &image.raw[446..462];
    assert_eq!(entry[0], 0x80, "partition is not bootable");
    assert_eq!(entry[4], 0x06, "partition type is not FAT16");
    assert_eq!(u32::from_le_bytes(entry[8..12].try_into().unwrap()), 2048);
    assert_eq!(
        u32::from_le_bytes(entry[12..16].try_into().unwrap()),
        63_488
    );

    let boot = &image.boot;
    assert_eq!(boot.layout, BootLayout::MbrPartitioned);
    assert_eq!(boot.fat_type, Some(FatType::Fat16));
    assert_eq!(boot.volume_label.as_deref(), Some("DOSKER"));
    assert_eq!(boot.media_descriptor, Some(0xF8));
    assert_eq!(boot.partition_start, Some(2048));
    assert_eq!(boot.partition_sectors, Some(63_488));
    assert_eq!(
        boot.geometry,
        Geometry {
            cylinders: 124,
            heads: 16,
            sectors: 32,
            sector_size: 512,
        }
    );

    assert_eq!(
        u32::from_le_bytes(
            image.raw[PARTITION_OFFSET + 28..PARTITION_OFFSET + 32]
                .try_into()
                .unwrap()
        ),
        2048
    );
    assert_eq!(
        u16::from_le_bytes(
            image.raw[PARTITION_OFFSET + 19..PARTITION_OFFSET + 21]
                .try_into()
                .unwrap()
        ),
        63_488
    );
}

#[test]
fn build_copies_the_pinned_mbr() {
    let image = build_base_image(&binaries(), &version()).unwrap();

    assert_eq!(&image.raw[..512], MBR_TEMPLATE);
}

#[test]
fn build_keeps_the_freedos_boot_loader() {
    let image = build_base_image(&binaries(), &version()).unwrap();
    let boot = &image.raw[PARTITION_OFFSET..PARTITION_OFFSET + 512];

    assert_eq!(&boot[..0x0B], &BOOT_SECTOR_TEMPLATE[..0x0B]);
    assert_eq!(&boot[0x3E..], &BOOT_SECTOR_TEMPLATE[0x3E..]);
    assert_eq!(&boot[510..512], &[0x55, 0xAA]);
}

#[test]
fn build_writes_the_expected_tree() {
    let image = build_base_image(&binaries(), &version()).unwrap();

    let paths: Vec<&str> = image.tree.iter().map(|(path, _)| path.as_str()).collect();
    assert_eq!(
        paths,
        vec![
            "AUTOEXEC.BAT",
            "COMMAND.COM",
            "CONFIG.SYS",
            "DOSKER",
            "DOSKER/DOSKER.EXE",
            "KERNEL.SYS",
        ]
    );

    assert_eq!(file_bytes(&image, "KERNEL.SYS"), KERNEL);
    assert_eq!(file_bytes(&image, "COMMAND.COM"), COMMAND);
    assert_eq!(file_bytes(&image, "CONFIG.SYS"), CONFIG_SYS_BYTES);
    assert_eq!(file_bytes(&image, "AUTOEXEC.BAT"), BASE_AUTOEXEC_BYTES);
    assert_eq!(file_bytes(&image, "DOSKER/DOSKER.EXE"), AGENT);

    for (path, node) in image.tree.iter() {
        assert_eq!(node.timestamp(), fixed_timestamp(), "timestamp at {path}");
        assert_eq!(
            *node.attributes(),
            Attributes::default(),
            "attributes at {path}"
        );
    }
}

#[test]
fn build_writes_the_config_sys_settings() {
    let image = build_base_image(&binaries(), &version()).unwrap();
    let bytes = file_bytes(&image, "CONFIG.SYS");

    assert_eq!(bytes, CONFIG_SYS_BYTES);
    assert_eq!(
        bytes,
        b"FILES=20\r\nBUFFERS=10\r\nLASTDRIVE=Z\r\n".as_slice()
    );
}

#[test]
fn build_writes_an_autoexec_that_starts_the_agent() {
    let image = build_base_image(&binaries(), &version()).unwrap();
    let bytes = file_bytes(&image, "AUTOEXEC.BAT");

    assert_eq!(bytes, BASE_AUTOEXEC_BYTES);
    assert_eq!(
        bytes,
        b"@ECHO OFF\r\nSET COMSPEC=C:\\COMMAND.COM\r\nC:\\DOSKER\\DOSKER.EXE\r\n".as_slice()
    );
}

#[test]
fn build_reports_a_mismatched_mbr() {
    let mut binaries = binaries();
    binaries.mbr[446 + 4] = 0x07;

    match build_base_image(&binaries, &version()) {
        Err(ImageError::FreedosTemplate { reason }) => {
            assert!(reason.contains("partition entry"), "reason: {reason}");
        }
        other => panic!("expected FreedosTemplate, got {other:?}"),
    }
}

#[test]
fn build_reports_a_short_boot_template() {
    let mut binaries = binaries();
    binaries.boot_sector.truncate(100);

    assert!(matches!(
        build_base_image(&binaries, &version()),
        Err(ImageError::FreedosTemplate { .. })
    ));
}
