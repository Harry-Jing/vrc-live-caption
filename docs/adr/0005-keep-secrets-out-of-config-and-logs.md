# Keep secrets out of config and logs

Service credentials stay outside ordinary configuration, logs, diagnostics,
and frontend-readable state. The frontend can save, delete, and check a
credential's status, but cannot read the plaintext back. This keeps
configuration and copied diagnostics safe to share and prevents presentation
state from becoming a bearer-secret read path.
