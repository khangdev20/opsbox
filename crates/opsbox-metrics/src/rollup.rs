//! Minute and hour averages for SQLite (docs/BACKEND_SPEC.md 7.3).

use opsbox_store::{Granularity, MetricRow};

use crate::engine::{DiskView, Point};

#[derive(Default)]
struct Acc {
    start: i64,
    n: u32,
    cpu: f64,
    cpu_max: f64,
    mem: f64,
    net_in: f64,
    net_out: f64,
    load: f64,
    disk: String,
}

impl Acc {
    fn finish(&self) -> MetricRow {
        let n = f64::from(self.n.max(1));
        MetricRow {
            ts: self.start,
            cpu: self.cpu / n,
            cpu_max: self.cpu_max,
            mem: self.mem / n,
            disk: self.disk.clone(),
            net_in: self.net_in / n,
            net_out: self.net_out / n,
            load: self.load / n,
        }
    }
}

fn disk_json(disks: &[DiskView]) -> String {
    let rows: Vec<_> = disks
        .iter()
        .map(|d| serde_json::json!({ "mount": d.mount, "used": d.used, "total": d.total }))
        .collect();
    serde_json::Value::Array(rows).to_string()
}

/// Accumulates 1 s points into minutes, and finished minutes into hours. Rows are
/// emitted when the *next* period starts, so a period is written once, complete.
#[derive(Default)]
pub struct Rollup {
    minute: Option<Acc>,
    hour: Option<Acc>,
}

impl Rollup {
    pub fn push(&mut self, p: &Point, disks: &[DiskView]) -> Vec<(Granularity, MetricRow)> {
        let mut out = Vec::new();
        let start = p.ts - p.ts.rem_euclid(60);
        if self.minute.as_ref().is_some_and(|m| m.start != start) {
            let done = self.minute.take().expect("checked").finish();
            self.add_to_hour(&done, &mut out);
            out.insert(0, (Granularity::Minute, done));
        }
        let m = self.minute.get_or_insert_with(|| Acc {
            start,
            ..Acc::default()
        });
        m.n += 1;
        m.cpu += f64::from(p.cpu);
        m.cpu_max = m.cpu_max.max(f64::from(p.cpu));
        m.mem += f64::from(p.mem_pct());
        m.net_in += p.net_in;
        m.net_out += p.net_out;
        m.load += f64::from(p.load[0]);
        m.disk = disk_json(disks);
        out
    }

    fn add_to_hour(&mut self, minute: &MetricRow, out: &mut Vec<(Granularity, MetricRow)>) {
        let start = minute.ts - minute.ts.rem_euclid(3600);
        if self.hour.as_ref().is_some_and(|h| h.start != start) {
            out.push((
                Granularity::Hour,
                self.hour.take().expect("checked").finish(),
            ));
        }
        let h = self.hour.get_or_insert_with(|| Acc {
            start,
            ..Acc::default()
        });
        h.n += 1;
        h.cpu += minute.cpu;
        h.cpu_max = h.cpu_max.max(minute.cpu_max);
        h.mem += minute.mem;
        h.net_in += minute.net_in;
        h.net_out += minute.net_out;
        h.load += minute.load;
        h.disk.clone_from(&minute.disk);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(ts: i64, cpu: f32) -> Point {
        Point {
            ts,
            cpu,
            cores: vec![],
            mem_total: 100,
            mem_used: 40,
            cache: 0,
            swap_total: 0,
            swap_used: 0,
            load: [1.0, 0.0, 0.0],
            net_in: 10.0,
            net_out: 20.0,
            disk_read: 0.0,
            disk_write: 0.0,
            conns: 0,
        }
    }
    fn disks(used: u64) -> Vec<DiskView> {
        vec![DiskView {
            mount: "/".into(),
            fstype: "ext4".into(),
            used,
            total: 100,
        }]
    }

    #[test]
    fn a_minute_is_written_once_when_the_next_one_starts() {
        let mut r = Rollup::default();
        for s in 0..60 {
            assert!(
                r.push(
                    &point(120 + s, if s < 30 { 10.0 } else { 30.0 }),
                    &disks(50)
                )
                .is_empty(),
                "second {s}"
            );
        }
        let out = r.push(&point(180, 99.0), &disks(60));
        assert_eq!(out.len(), 1);
        let (g, row) = &out[0];
        assert_eq!((*g, row.ts), (Granularity::Minute, 120));
        assert!(
            (row.cpu - 20.0).abs() < 1e-9 && (row.cpu_max - 30.0).abs() < 1e-9,
            "{row:?}"
        );
        assert_eq!(
            (row.mem, row.net_in, row.net_out, row.load),
            (40.0, 10.0, 20.0, 1.0)
        );
        assert_eq!(row.disk, r#"[{"mount":"/","total":100,"used":50}]"#);
    }

    #[test]
    fn gaps_do_not_invent_rows() {
        let mut r = Rollup::default();
        r.push(&point(0, 10.0), &disks(1));
        // Nothing for ten minutes (the sampler was down), then a point: one row for the minute that had data.
        let out = r.push(&point(600, 20.0), &disks(1));
        assert_eq!(out.iter().map(|(_, r)| r.ts).collect::<Vec<_>>(), [0]);
    }

    #[test]
    fn minutes_roll_up_into_an_hour() {
        let mut r = Rollup::default();
        let mut hours = Vec::new();
        for minute in 0..125 {
            for (g, row) in r.push(&point(minute * 60, minute as f32), &disks(1)) {
                if g == Granularity::Hour {
                    hours.push(row);
                }
            }
        }
        assert_eq!(
            hours.len(),
            2,
            "hours 0 and 1 are complete; hour 2 is still open"
        );
        assert_eq!(hours[0].ts, 0);
        assert_eq!(hours[1].ts, 3600);
        assert!((hours[0].cpu - 29.5).abs() < 1e-9, "mean of 0..=59");
        assert!((hours[0].cpu_max - 59.0).abs() < 1e-9);
    }
}
