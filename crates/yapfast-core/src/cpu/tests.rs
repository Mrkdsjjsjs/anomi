//! Auto's choice on the layouts of real processors, as Linux numbers them,
//! and the layout read from sysfs files written as Linux writes them.

use super::*;

/// The ranks amd-pstate gives this 9950X3D's cores in its "frequency" mode,
/// by core: the V-cache chiplet's 0-7 below the other's 8-15.
const RANKS_9950X3D: [u32; 16] = [
    186, 196, 171, 201, 181, 191, 166, 176, 221, 231, 216, 236, 226, 236, 206, 211,
];

/// A processor of `domains`, each an L3 domain's physical cores and L3
/// size in KiB, cores numbered on from domain to domain; with `smt`, core
/// n's sibling is CPU n plus the number of cores, as on AMD's. `ranks` by
/// core, where given.
fn machine(domains: &[(u32, u32)], smt: bool, ranks: &[u32]) -> Topology {
    let total: u32 = domains.iter().map(|(cores, _)| cores).sum();
    let mut cpus = Vec::new();
    let mut core = 0;
    for (l3, &(cores, l3_kib)) in (0..).zip(domains) {
        for _ in 0..cores {
            let rank = ranks.get(core as usize).copied();
            for logical in [Some(core), smt.then_some(core + total)]
                .into_iter()
                .flatten()
            {
                cpus.push(Cpu {
                    logical,
                    core,
                    l3,
                    l3_kib,
                    rank,
                    small: false,
                });
            }
            core += 1;
        }
    }
    cpus.sort_by_key(|cpu| cpu.logical);
    let allowed = cpus.iter().map(|cpu| cpu.logical).collect();
    Topology { cpus, allowed }
}

/// This machine: a Ryzen 9 9950X3D, its V-cache chiplet first.
fn ryzen_9950x3d() -> Topology {
    machine(&[(8, 98_304), (8, 32_768)], true, &RANKS_9950X3D)
}

/// An i9-13900K: 8 performance cores with Hyper-Threading, their siblings
/// side by side, then 16 efficiency cores, one L3.
fn intel_13900k() -> Topology {
    let mut cpus = Vec::new();
    for core in 0..8 {
        for logical in [2 * core, 2 * core + 1] {
            cpus.push(Cpu {
                logical,
                core,
                l3: 0,
                l3_kib: 36_864,
                rank: None,
                small: false,
            });
        }
    }
    for core in 8..24 {
        cpus.push(Cpu {
            logical: core + 8,
            core,
            l3: 0,
            l3_kib: 36_864,
            rank: None,
            small: true,
        });
    }
    let allowed = cpus.iter().map(|cpu| cpu.logical).collect();
    Topology { cpus, allowed }
}

/// What Auto picks, as Linux lists CPUs; "all" for none.
fn auto_picks(topology: &Topology) -> String {
    choose(topology, &CpuMode::Auto).map_or_else(|| "all".to_owned(), |mask| mask.to_string())
}

#[test]
fn this_9950x3d_gets_the_chiplet_with_less_cache() {
    assert_eq!(auto_picks(&ryzen_9950x3d()), "8-15,24-31");
}

#[test]
fn a_9950x_gets_the_chiplet_without_core_0() {
    let ranks = [
        231, 226, 236, 221, 216, 211, 206, 201, 196, 191, 186, 181, 176, 171, 166, 161,
    ];
    let topology = machine(&[(8, 32_768), (8, 32_768)], true, &ranks);
    assert_eq!(auto_picks(&topology), "8-15,24-31");
}

#[test]
fn a_7900x3d_gets_its_six_core_chiplet_whole() {
    let topology = machine(&[(6, 98_304), (6, 32_768)], true, &[]);
    assert_eq!(auto_picks(&topology), "6-11,18-23");
}

#[test]
fn equal_chiplets_go_by_their_best_rank_then_the_last() {
    // A Threadripper's four chiplets: the second's best core ranks lowest.
    let mut ranks: Vec<u32> = (0..32).map(|core| 200 + core).collect();
    ranks[8..16].iter_mut().for_each(|rank| *rank -= 100);
    let ranked = machine(&[(8, 32_768); 4], true, &ranks);
    assert_eq!(auto_picks(&ranked), "8-15,40-47");
    let unranked = machine(&[(8, 32_768); 4], true, &[]);
    assert_eq!(auto_picks(&unranked), "24-31,56-63");
}

#[test]
fn a_13900k_gets_its_efficiency_cores() {
    assert_eq!(auto_picks(&intel_13900k()), "16-31");
}

#[test]
fn strix_point_gets_its_smaller_zen_5c_cores() {
    // A Ryzen AI 9 HX 370: 4 Zen 5 cores with 16 MB, 8 Zen 5c with 8 MB.
    let topology = machine(&[(4, 16_384), (8, 8_192)], true, &[]);
    assert_eq!(auto_picks(&topology), "4-11,16-23");
}

#[test]
fn a_5800x_gets_its_last_two_cores() {
    let topology = machine(&[(8, 32_768)], true, &[]);
    assert_eq!(auto_picks(&topology), "6-7,14-15");
}

#[test]
fn the_last_cores_are_the_lowest_ranked_but_never_core_0() {
    // Core 0 ranks lowest of all, then cores 3 and 1.
    let ranks = [100, 180, 236, 175, 221, 231, 226, 211];
    let topology = machine(&[(8, 32_768)], true, &ranks);
    assert_eq!(auto_picks(&topology), "1,3,9,11");
}

#[test]
fn a_3600_gets_the_last_cores_of_its_second_ccx() {
    // Two CCXs of three cores, too small to take whole.
    let topology = machine(&[(3, 16_384), (3, 16_384)], true, &[]);
    assert_eq!(auto_picks(&topology), "4-5,10-11");
}

#[test]
fn a_four_core_eight_thread_processor_gives_one_core() {
    let topology = machine(&[(4, 8_192)], true, &[]);
    assert_eq!(auto_picks(&topology), "3,7");
}

#[test]
fn small_processors_are_left_whole() {
    assert_eq!(auto_picks(&machine(&[(4, 6_144)], false, &[])), "all");
    assert_eq!(auto_picks(&machine(&[(2, 4_096)], true, &[])), "all");
    assert_eq!(auto_picks(&machine(&[(3, 4_096)], false, &[])), "all");
}

#[test]
fn larger_single_chiplets_give_a_quarter() {
    assert_eq!(auto_picks(&machine(&[(6, 9_216)], false, &[])), "4-5");
    assert_eq!(
        auto_picks(&machine(&[(12, 32_768)], true, &[])),
        "9-11,21-23"
    );
    assert_eq!(
        auto_picks(&machine(&[(16, 32_768)], true, &[])),
        "12-15,28-31"
    );
    assert_eq!(
        [4, 5, 6, 8, 12, 16, 64].map(last_count),
        [1, 2, 2, 2, 3, 4, 16]
    );
}

#[test]
fn a_process_someone_else_narrowed_is_left_as_it_is() {
    let mut topology = ryzen_9950x3d();
    topology.allowed = CpuMask::parse("0-15").unwrap();
    assert_eq!(auto_picks(&topology), "all");
    let chosen = choose(&topology, &CpuMode::Cores(vec![8, 9]));
    assert_eq!(chosen.map(|mask| mask.to_string()).as_deref(), Some("8-9"));
}

#[test]
fn all_and_chosen_cores() {
    let topology = ryzen_9950x3d();
    assert_eq!(choose(&topology, &CpuMode::All), None);
    let chosen = choose(&topology, &CpuMode::Cores(vec![8, 9]));
    assert_eq!(
        chosen.map(|mask| mask.to_string()).as_deref(),
        Some("8-9,24-25")
    );
    assert_eq!(choose(&topology, &CpuMode::Cores(vec![99])), None);
    assert_eq!(choose(&topology, &CpuMode::Cores(Vec::new())), None);
    assert_eq!(choose(&Topology::default(), &CpuMode::Auto), None);
}

#[test]
fn describes_the_processor_and_the_cores_used() {
    let topology = ryzen_9950x3d();
    let auto = choose(&topology, &CpuMode::Auto);
    assert_eq!(
        describe(&topology, auto.as_ref()),
        "Chiplets: 2, cores: 16, threads: 32. \
         Runs on cores 8-15 (CPUs 8-15, 24-31): the chiplet with the smaller cache, 32 MB."
    );
    assert_eq!(
        describe(&topology, None),
        "Chiplets: 2, cores: 16, threads: 32. Runs on every core."
    );
    let chosen = choose(&topology, &CpuMode::Cores(vec![2, 3]));
    assert_eq!(
        describe(&topology, chosen.as_ref()),
        "Chiplets: 2, cores: 16, threads: 32. Runs on cores 2-3 (CPUs 2-3, 18-19), as chosen."
    );
    let intel = intel_13900k();
    assert_eq!(
        describe(&intel, choose(&intel, &CpuMode::Auto).as_ref()),
        "Performance cores: 8, efficiency cores: 16. \
         Runs on cores 8-23 (CPUs 16-31): the efficiency cores."
    );
    let small = machine(&[(4, 6_144)], false, &[]);
    assert_eq!(
        describe(&small, None),
        "Cores: 4, threads: 4. Runs on every core: too few to set some aside."
    );
    let plain = machine(&[(8, 32_768)], true, &[]);
    assert_eq!(
        describe(&plain, choose(&plain, &CpuMode::Auto).as_ref()),
        "Cores: 8, threads: 16. Runs on cores 6-7 (CPUs 6-7, 14-15): the last ones, core 0 stays free."
    );
    let mut narrowed = ryzen_9950x3d();
    narrowed.allowed = CpuMask::parse("0-7").unwrap();
    assert_eq!(
        describe(&narrowed, None),
        "Chiplets: 2, cores: 16, threads: 32. Runs on the CPUs the system gave it: 0-7."
    );
    assert_eq!(
        describe(&Topology::default(), None),
        "Could not read the processor's cores."
    );
}

#[test]
fn equal_chiplets_are_described_by_core_0() {
    let topology = machine(&[(8, 32_768), (8, 32_768)], true, &[]);
    assert_eq!(
        describe(&topology, choose(&topology, &CpuMode::Auto).as_ref()),
        "Chiplets: 2, cores: 16, threads: 32. \
         Runs on cores 8-15 (CPUs 8-15, 24-31): the chiplet without core 0."
    );
}

#[test]
fn modes_read_and_write_as_layout_json_keeps_them() {
    let written = |mode: &CpuMode| serde_json::to_string(mode).unwrap();
    assert_eq!(written(&CpuMode::Auto), r#""auto""#);
    assert_eq!(written(&CpuMode::All), r#""all""#);
    assert_eq!(written(&CpuMode::Cores(vec![8, 9])), r#"{"cores":[8,9]}"#);
    let read = |text: &str| serde_json::from_str::<CpuMode>(text).unwrap();
    assert_eq!(read(r#""auto""#), CpuMode::Auto);
    assert_eq!(read(r#""all""#), CpuMode::All);
    assert_eq!(read(r#"{"cores":[8,9]}"#), CpuMode::Cores(vec![8, 9]));
    // What a later version might write reads as Auto.
    assert_eq!(read(r#""while-gaming""#), CpuMode::Auto);
    assert_eq!(read(r#"{"cores":"8-15"}"#), CpuMode::Auto);
    assert_eq!(read(r#"{"cores":[]}"#), CpuMode::Auto);
    assert_eq!(read("7"), CpuMode::Auto);
}

#[test]
fn masks_read_and_write_as_linux_lists_cpus() {
    let mask = CpuMask::parse("0-7,16-23\n").unwrap();
    assert_eq!(mask.len(), 16);
    assert_eq!(mask.to_string(), "0-7,16-23");
    assert_eq!(mask.spaced(), "0-7, 16-23");
    assert_eq!(CpuMask::parse("1,3,5").unwrap().to_string(), "1,3,5");
    assert!(CpuMask::parse("").unwrap().is_empty());
    assert_eq!(CpuMask::parse("5-2"), None);
    assert_eq!(CpuMask::parse("x"), None);
    assert_eq!(CpuMask::parse("1024"), None);
    let json = serde_json::to_string(&mask).unwrap();
    assert_eq!(json, r#""0-7,16-23""#);
    assert_eq!(serde_json::from_str::<CpuMask>(&json).unwrap(), mask);
    let both = mask.intersection(&CpuMask::parse("4-19").unwrap());
    assert_eq!(both.to_string(), "4-7,16-19");
}

/// A stream's x264, and the decoders, take a thread for each logical CPU
/// of the mask chosen, at most 4, never one for each of the machine's.
#[test]
fn threads_follow_the_cpus_chosen() {
    let topology = ryzen_9950x3d();
    let threads = |mode: CpuMode| choose(&topology, &mode).map(|mask| threads_on(mask.len()));
    // A core and its sibling, then three cores.
    assert_eq!(threads(CpuMode::Cores(vec![15])), Some(2));
    assert_eq!(threads(CpuMode::Cores(vec![13, 14, 15])), Some(4));
    assert_eq!(threads(CpuMode::Auto), Some(4), "at most 4");
    // All: every CPU the process may use, as the system counts them.
    assert_eq!(threads(CpuMode::All), None);
    assert_eq!(threads_on(32), 4);
    assert_eq!(threads_on(0), 1, "never none");
}

#[test]
fn numbering_follows_the_first_logical_cpus() {
    // As Linux names them: each core and domain by its first CPU.
    let mut cpus: Vec<Cpu> = [(16, 0, 0), (0, 0, 0), (8, 8, 8), (24, 8, 8)]
        .map(|(logical, core, l3)| Cpu {
            logical,
            core,
            l3,
            l3_kib: 0,
            rank: None,
            small: false,
        })
        .to_vec();
    number(&mut cpus);
    let numbered: Vec<(u32, u32, u32)> = cpus
        .iter()
        .map(|cpu| (cpu.logical, cpu.core, cpu.l3))
        .collect();
    assert_eq!(numbered, [(0, 0, 0), (8, 1, 1), (16, 0, 0), (24, 1, 1)]);
}

#[cfg(target_os = "linux")]
mod sysfs {
    use std::path::Path;

    use super::*;

    fn write(root: &Path, path: &str, text: &str) {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, format!("{text}\n")).unwrap();
    }

    /// The caches of CPU `cpu`: L1 and L2 of its own, and the L3 it shares.
    fn caches(root: &Path, cpu: u32, own: &str, l3: &str, l3_size: &str) {
        let cache = format!("devices/system/cpu/cpu{cpu}/cache");
        for (index, level) in [(0, "1"), (1, "1"), (2, "2")] {
            write(root, &format!("{cache}/index{index}/level"), level);
            write(root, &format!("{cache}/index{index}/shared_cpu_list"), own);
            write(root, &format!("{cache}/index{index}/size"), "1024K");
        }
        write(root, &format!("{cache}/index3/level"), "3");
        write(root, &format!("{cache}/index3/shared_cpu_list"), l3);
        write(root, &format!("{cache}/index3/size"), l3_size);
    }

    fn read(root: &Path) -> Vec<Cpu> {
        let mut cpus = super::super::linux::read(root);
        number(&mut cpus);
        cpus
    }

    #[test]
    fn reads_this_9950x3d() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path();
        write(root, "devices/system/cpu/online", "0-31");
        write(root, "devices/system/cpu/isolated", "");
        for cpu in 0..32 {
            let core = cpu % 16;
            let siblings = format!("{core},{}", core + 16);
            let dir = format!("devices/system/cpu/cpu{cpu}");
            write(root, &format!("{dir}/topology/core_cpus_list"), &siblings);
            let (l3, size) = if core < 8 {
                ("0-7,16-23", "98304K")
            } else {
                ("8-15,24-31", "32768K")
            };
            caches(root, cpu, &siblings, l3, size);
            let rank = RANKS_9950X3D[core as usize].to_string();
            write(
                root,
                &format!("{dir}/cpufreq/amd_pstate_prefcore_ranking"),
                &rank,
            );
        }
        assert_eq!(read(root), ryzen_9950x3d().cpus);
    }

    #[test]
    fn reads_a_13900k() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path();
        write(root, "devices/system/cpu/online", "0-31");
        write(root, "devices/cpu_core/cpus", "0-15");
        write(root, "devices/cpu_atom/cpus", "16-31");
        for cpu in 0..32 {
            let siblings = if cpu < 16 {
                format!("{}-{}", cpu / 2 * 2, cpu / 2 * 2 + 1)
            } else {
                cpu.to_string()
            };
            let dir = format!("devices/system/cpu/cpu{cpu}");
            write(
                root,
                &format!("{dir}/topology/thread_siblings_list"),
                &siblings,
            );
            caches(root, cpu, &siblings, "0-31", "36864K");
        }
        assert_eq!(read(root), intel_13900k().cpus);
    }

    #[test]
    fn leaves_out_cpus_offline_or_isolated() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path();
        write(root, "devices/system/cpu/online", "0-2");
        write(root, "devices/system/cpu/isolated", "1");
        for cpu in 0..4 {
            let dir = format!("devices/system/cpu/cpu{cpu}");
            write(
                root,
                &format!("{dir}/topology/core_cpus_list"),
                &cpu.to_string(),
            );
            write(root, &format!("{dir}/cpufreq/cpuinfo_max_freq"), "4200000");
        }
        let cpus = read(root);
        let logical: Vec<u32> = cpus.iter().map(|cpu| cpu.logical).collect();
        assert_eq!(logical, [0, 2]);
        // No L3 named: one domain, of unknown size; the clock as the rank.
        assert!(
            cpus.iter()
                .all(|cpu| (cpu.l3, cpu.l3_kib, cpu.rank) == (0, 0, Some(4_200_000)))
        );
    }
}
