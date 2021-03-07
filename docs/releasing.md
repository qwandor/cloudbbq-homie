# Release procedure

To release a new version of `cloudbbq-homie`:

1. Increment the version number in [Cargo.toml](Cargo.toml), and push to main.
2. Run `cargo publish --dry-run`.
3. Tag the commit which merges this to `main` to match the new version, like `x.y.z`, and push to
   the repository.
4. Wait for the
   [Package workflow](https://github.com/qwandor/cloudbbq-homie/actions?query=workflow%3APackage) to
   create a new draft [release](https://github.com/qwandor/cloudbbq-homie/releases) including the
   Debian packages.
5. Run `cargo publish`.
6. Edit the release, add an appropriate description, and then publish it.
