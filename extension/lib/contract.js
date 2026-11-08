// Generated from contracts/snapshot-v1.schema.json; do not edit.
export const PROTOCOL_VERSION = 1;
export const MAX_LINE = 4096;
export const MAX_INTERVAL = 60;
export const REALTIME_MS = 500;
export const METRICS = {
  "cpu": {
    "temp": "number",
    "usage": "percent"
  },
  "disk": {
    "read": "integer",
    "temp": "number",
    "write": "integer"
  },
  "gpu": {
    "memory": "percent",
    "temp": "number",
    "usage": "percent"
  },
  "memory": {
    "swap": "percent",
    "used": "percent"
  },
  "net": {
    "rx": "integer",
    "tx": "integer"
  }
};
