//! Everything the Task Manager knows about the machine, read straight from
//! `/proc` and `/sys` (plus `systemctl` for services).

pub mod apps;
pub mod autostart;
pub mod desktop;
pub mod kill;
pub mod process;
pub mod procfs;
pub mod services;
pub mod system;
pub mod toplevel;

use std::collections::HashMap;
use std::time::Instant;

use apps::Grouping;
use desktop::AppIndex;
use process::{Process, ProcessSampler};
use system::System;
use toplevel::Toplevels;

pub struct Sampler {
    pub system: System,
    pub procs: Vec<Process>,
    pub grouping: Grouping,
    pub index: AppIndex,
    pub toplevels: Toplevels,
    users: HashMap<u32, String>,
    sampler: ProcessSampler,
    by_pid: HashMap<i32, usize>,
}

impl Sampler {
    pub fn new(toplevels: Toplevels) -> Self {
        let users = std::fs::read_to_string("/etc/passwd").map(|s| procfs::parse_passwd(&s)).unwrap_or_default();
        Self {
            system: System::new(),
            procs: Vec::new(),
            grouping: Grouping::default(),
            index: AppIndex::load(),
            toplevels,
            users,
            sampler: ProcessSampler::new("/proc"),
            by_pid: HashMap::new(),
        }
    }

    pub fn sample(&mut self, now: Instant) {
        self.system.sample(now);
        self.procs = self.sampler.sample(self.system.machine_ticks, now);
        self.by_pid = self.procs.iter().enumerate().map(|(i, p)| (p.pid, i)).collect();
        self.regroup();
    }

    /// Re-sorts processes into apps and background (the window list changed).
    pub fn regroup(&mut self) {
        let windows = self.toplevels.get();
        self.grouping = apps::group(&self.procs, &self.index, windows.as_deref());
    }

    pub fn process(&self, pid: i32) -> Option<&Process> {
        self.by_pid.get(&pid).map(|&i| &self.procs[i])
    }

    pub fn user_name(&self, uid: u32) -> String {
        self.users.get(&uid).cloned().unwrap_or_else(|| uid.to_string())
    }

    /// `pid` and everything it started, parents first.
    pub fn tree(&self, pid: i32) -> Vec<i32> {
        let mut children: HashMap<i32, Vec<i32>> = HashMap::new();
        for p in &self.procs {
            children.entry(p.ppid).or_default().push(p.pid);
        }
        let mut out = vec![pid];
        let mut i = 0;
        while i < out.len() {
            let cur = out[i];
            if let Some(c) = children.get(&cur) {
                out.extend(c.iter().filter(|&&c| c != cur));
            }
            i += 1;
        }
        out
    }

    pub fn target(&self, pid: i32) -> Option<kill::Target> {
        self.process(pid).map(|p| kill::Target { pid, start_ticks: p.start_ticks })
    }
}
