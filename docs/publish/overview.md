# Publish Overview

## Current source candidate

The active workspace contains one package:

```text
pulith 0.1.0
```

The previous `pulith-*` side crates were published historically, but they are not current source packages and must not be used as the release graph for this repository revision.

## Release policy

The consolidated crate is not release-ready merely because historical side crates were published. A release requires:

1. feature graph and package metadata review;
2. smallest-feature compilation checks;
3. all-feature fmt, clippy, test, and rustdoc gates;
4. security/dependency review;
5. public API and SemVer review against historical names;
6. crates.io-targeted dry run from a clean commit;
7. explicit go/no-go decision in the readiness matrix.

Operational status is recorded in [`readiness-matrix.md`](readiness-matrix.md). The executable gate list is [`checklist.md`](checklist.md).
