-- Phase 1 / M3: metric rollups (docs/BACKEND_SPEC.md 5 and 7.3). One row per minute is
-- kept for 7 days and one per hour for 90 days; the live 1 s data stays in memory.
-- `ts` is the start of the period in unix seconds.

CREATE TABLE metrics_minute (
    ts       INTEGER PRIMARY KEY NOT NULL,
    cpu      REAL NOT NULL,      -- mean total CPU %
    cpu_max  REAL NOT NULL,      -- highest 1 s value in the period
    mem      REAL NOT NULL,      -- mean memory used %
    disk     TEXT NOT NULL CHECK (json_valid(disk)),  -- [{"mount","used","total"}] at the end of the period
    net_in   REAL NOT NULL,      -- mean bytes/s
    net_out  REAL NOT NULL,
    load     REAL NOT NULL       -- mean 1-minute load
) STRICT;

CREATE TABLE metrics_hour (
    ts       INTEGER PRIMARY KEY NOT NULL,
    cpu      REAL NOT NULL,
    cpu_max  REAL NOT NULL,
    mem      REAL NOT NULL,
    disk     TEXT NOT NULL CHECK (json_valid(disk)),
    net_in   REAL NOT NULL,
    net_out  REAL NOT NULL,
    load     REAL NOT NULL
) STRICT;
