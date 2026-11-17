//! Metric rollups: one row per minute (kept 7 days) and per hour (kept 90 days).

use rusqlite::params;

use crate::{Store, StoreError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Granularity {
    Minute,
    Hour,
}

impl Granularity {
    fn table(self) -> &'static str {
        match self {
            Self::Minute => "metrics_minute",
            Self::Hour => "metrics_hour",
        }
    }
    /// How long rows of this kind are kept, in seconds.
    pub fn retention_secs(self) -> i64 {
        match self {
            Self::Minute => 7 * 24 * 3600,
            Self::Hour => 90 * 24 * 3600,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MetricRow {
    /// Start of the period, unix seconds.
    pub ts: i64,
    pub cpu: f64,
    pub cpu_max: f64,
    pub mem: f64,
    /// JSON: `[{"mount","used","total"}]`.
    pub disk: String,
    pub net_in: f64,
    pub net_out: f64,
    pub load: f64,
}

impl Store {
    /// Insert (or replace, if the period was written before a restart) one rollup row.
    pub fn put_metric(&self, g: Granularity, r: &MetricRow) -> Result<(), StoreError> {
        self.lock().execute(
            &format!(
                "INSERT OR REPLACE INTO {} (ts, cpu, cpu_max, mem, disk, net_in, net_out, load) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                g.table()
            ),
            params![r.ts, r.cpu, r.cpu_max, r.mem, r.disk, r.net_in, r.net_out, r.load],
        )?;
        Ok(())
    }

    /// Rows with `from <= ts < to`, oldest first.
    pub fn metrics_between(
        &self,
        g: Granularity,
        from: i64,
        to: i64,
    ) -> Result<Vec<MetricRow>, StoreError> {
        let conn = self.lock();
        let mut stmt = conn.prepare(&format!(
            "SELECT ts, cpu, cpu_max, mem, disk, net_in, net_out, load FROM {} WHERE ts >= ?1 AND ts < ?2 ORDER BY ts",
            g.table()
        ))?;
        let rows = stmt.query_map(params![from, to], |r| {
            Ok(MetricRow {
                ts: r.get(0)?,
                cpu: r.get(1)?,
                cpu_max: r.get(2)?,
                mem: r.get(3)?,
                disk: r.get(4)?,
                net_in: r.get(5)?,
                net_out: r.get(6)?,
                load: r.get(7)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Delete rows older than the retention limit; returns how many went.
    pub fn purge_metrics(&self, now: i64) -> Result<usize, StoreError> {
        let conn = self.lock();
        let mut n = 0;
        for g in [Granularity::Minute, Granularity::Hour] {
            n += conn.execute(
                &format!("DELETE FROM {} WHERE ts < ?1", g.table()),
                [now - g.retention_secs()],
            )?;
        }
        Ok(n)
    }
}
