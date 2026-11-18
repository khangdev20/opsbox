use opsbox_capacity::{requirements::*, *};
fn host(ram: u64, swap: u64, docker: bool) -> Snapshot {
    Snapshot {
        profile: Profile {
            memory_mb: ram,
            available_mb: ram,
            swap_mb: swap,
            swap_free_mb: swap,
            online_cpus: 1,
            cpu_millis: 1000,
            cgroup_v2: true,
            docker,
            disks: vec![Disk {
                path: "/".into(),
                total_mb: 20480,
                free_mb: 10000,
            }],
            ..Default::default()
        },
        reservations: vec![],
    }
}
fn status(f: Fit) -> &'static str {
    match f {
        Fit::Ok => "ok",
        Fit::Tight { .. } => "tight",
        Fit::Blocked { .. } => "blocked",
    }
}
#[test]
fn feature_table_for_small_servers_with_docker_and_swap() {
    // Every feature at 512 MB, 1/2/4 GB, Docker present/absent, swap present/absent.
    for ram in [512, 1024, 2048, 4096] {
        for swap in [0, 1024] {
            for docker in [false, true] {
                let s = host(ram, swap, docker);
                let physical =
                    ram.saturating_sub(os_mb(ram) + OPSBOX_MB + if docker { DOCKER_MB } else { 0 });
                for (feature, min, recommended, disk) in FEATURES {
                    let total = matches!(feature, Feature::Install | Feature::Docker);
                    let free = if total {
                        ram
                    } else if *feature == Feature::Database {
                        physical
                    } else {
                        physical + swap / 2
                    };
                    let expected = if free < *min || 10000 < DISK_MIN_MB + disk {
                        "blocked"
                    } else if free < *recommended {
                        "tight"
                    } else {
                        "ok"
                    };
                    assert_eq!(
                        status(s.admit(&Request {
                            feature: *feature,
                            ..Default::default()
                        })),
                        expected,
                        "{ram} MB + {swap} swap, docker={docker}, {feature:?}"
                    );
                }
            }
        }
    }
}
#[test]
fn reservations_are_real_and_removal_frees_memory() {
    let mut s = host(1024, 0, true);
    assert_eq!(s.budget(None).allocatable_mb, 554);
    for (name, memory) in [("api", 256), ("cache", 128)] {
        s.reservations.push(Reservation {
            name: name.into(),
            memory_mb: memory,
            cpu_millis: 100,
            database: false,
        });
    }
    assert_eq!(s.budget(None).allocatable_mb, 170);
    let req = Request {
        feature: Feature::Database,
        memory_mb: 256,
        ..Default::default()
    };
    let fit = s.admit(&req);
    assert!(fit.blocked());
    if let Fit::Blocked { suggestions, .. } = fit {
        assert!(
            suggestions
                .iter()
                .any(|s| s.contains("api") && s.contains("256"))
        );
    }
    s.reservations.remove(0);
    assert_eq!(s.budget(None).allocatable_mb, 426);
    assert!(!s.admit(&req).blocked());
}
#[test]
fn swap_never_counts_for_any_database_and_only_half_counts_for_apps() {
    let s = host(1024, 2048, true);
    assert_eq!(s.budget(None).allocatable_mb, 1578);
    assert_eq!(s.budget(None).database_allocatable_mb, 554);
    assert!(
        !s.admit(&Request {
            memory_mb: 1024,
            ..Default::default()
        })
        .blocked()
    );
    assert!(
        s.admit(&Request {
            feature: Feature::Database,
            memory_mb: 1024,
            ..Default::default()
        })
        .blocked()
    );
    assert!(
        s.admit(&Request {
            feature: Feature::Template,
            database: true,
            memory_mb: 1024,
            ..Default::default()
        })
        .blocked()
    );
}
#[test]
fn psi_fixture_requires_two_minutes_and_low_available_memory_blocks_immediately() {
    let psi = parse_psi(include_str!("fixtures/memory-pressure")).unwrap();
    assert_eq!(psi, 2150);
    let mut s = host(4096, 0, true);
    s.profile.memory_psi_avg60 = Some(psi);
    s.profile.pressure_seconds = 119;
    assert!(!s.admit(&Request::default()).blocked());
    s.profile.pressure_seconds = 120;
    assert!(s.admit(&Request::default()).blocked());
    s.profile.memory_psi_avg60 = None;
    s.profile.available_mb = OS_MIN_MB - 1;
    assert!(s.admit(&Request::default()).blocked());
    assert_eq!(parse_psi("full avg60=99.0"), None);
    assert_eq!(parse_psi("some avg60=NaN"), None);
}
#[test]
fn cpu_uses_quota_reserve_and_two_times_overcommit() {
    let mut s = host(4096, 0, true);
    s.profile.cpu_millis = 500;
    assert_eq!(s.budget(None).allocatable_cpu_millis, 300);
    assert!(
        !s.admit(&Request {
            cpu_millis: 300,
            ..Default::default()
        })
        .blocked()
    );
    assert!(
        s.admit(&Request {
            cpu_millis: 301,
            ..Default::default()
        })
        .blocked()
    );
}
#[test]
fn each_filesystem_keeps_its_reserve_and_image_size() {
    let mut s = host(4096, 0, true);
    s.profile.disks.push(Disk {
        path: "/docker".into(),
        total_mb: 100000,
        free_mb: 10100,
    });
    assert!(
        !s.admit(&Request {
            feature: Feature::ImagePull,
            disk_mb: 100,
            ..Default::default()
        })
        .blocked()
    );
    assert!(
        s.admit(&Request {
            feature: Feature::ImagePull,
            disk_mb: 101,
            ..Default::default()
        })
        .blocked()
    );
}
#[test]
fn no_cgroup_v2_blocks_every_container_even_when_ram_is_plentiful() {
    let mut s = host(4096, 1024, true);
    s.profile.cgroup_v2 = false;
    for feature in [
        Feature::App,
        Feature::Build,
        Feature::Database,
        Feature::Template,
    ] {
        assert!(
            s.admit(&Request {
                feature,
                ..Default::default()
            })
            .blocked()
        );
    }
}
#[test]
fn replace_removes_only_one_reservation_and_sums_cannot_overflow() {
    let mut s = host(4096, 0, true);
    for _ in 0..2 {
        s.reservations.push(Reservation {
            name: "api".into(),
            memory_mb: 256,
            cpu_millis: 0,
            database: false,
        });
    }
    assert_eq!(s.budget(Some("api")).reserved_mb, 256);
    s.reservations.push(Reservation {
        name: "unlimited".into(),
        memory_mb: u64::MAX,
        cpu_millis: u64::MAX,
        database: false,
    });
    assert!(s.admit(&Request::default()).blocked());
    assert!(s.over_capacity());
}
#[test]
fn swap_offer_and_meminfo_units() {
    assert_eq!(host(1024, 0, false).swap_offer_mb(), Some(1024));
    let mut s = host(2048, 0, false);
    s.profile.disks[0].free_mb = 20481;
    assert_eq!(s.swap_offer_mb(), Some(2048));
    assert_eq!(host(4096, 0, false).swap_offer_mb(), None);
    assert_eq!(host(1024, 512, false).swap_offer_mb(), None);
    assert_eq!(
        meminfo_mb("MemTotal: 1048576 kB\nSwapFree: 524288 kB\n", "MemTotal"),
        1024
    );
}
