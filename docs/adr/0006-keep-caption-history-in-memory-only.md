# Keep caption history in memory only

The app keeps bounded recent caption and diagnostic state in memory and creates
no automatic history or report files. Explicitly copied diagnostic reports are
allowlisted and exclude user content, credentials and their status,
configuration, device identity, network targets, filesystem paths, and
diagnostic free text.

Persistent history or export requires a new retention and privacy design.
