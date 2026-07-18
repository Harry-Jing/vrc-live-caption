# Keep secrets out of config and logs

Date: 2026-05

API keys live in the operating system credential store (with an
environment-variable fallback for the OpenAI key), never in ordinary config
files or logs. The frontend can save, delete, and check the status of a
secret, but can never read the plaintext back. This keeps config files and
diagnostics safe to share.
