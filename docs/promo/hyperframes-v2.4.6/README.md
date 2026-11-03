# Historical promo source snapshot

This directory preserves the six existing authoring files from the local
v2.4.6 promo workspace. It is a partial source snapshot, not a verified render.
The `compositions/` and `assets/` directories declared by `hyperframes.json`
are absent from this snapshot, and no dependency lockfile was available.

Recover or recreate the required assets before claiming a reproducible render.
The pinned HyperFrames command and the external GSAP URL still require network
access. These promo files are independent from the application's build; use
the repository's [development setup](../../dev-setup.md) for application source.
