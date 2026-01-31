# AUR Release Process

Steps to publish a new version to the Arch User Repository.

## Prerequisites

- AUR package repository cloned at `~/aur/ralph/`
- SSH access to AUR configured

## Steps

1. **Bump version in Cargo.toml**
   ```bash
   # Edit version in Cargo.toml
   cargo update  # Update Cargo.lock
   ```

2. **Commit, tag, and push**
   ```bash
   git add Cargo.toml Cargo.lock
   git commit -m "chore: bump version to X.Y.Z"
   git tag vX.Y.Z
   git push && git push --tags
   ```

3. **Reset AUR repo to upstream**
   ```bash
   cd ~/aur/ralph
   git fetch origin
   git reset --hard origin/master
   ```

4. **Get new tarball checksum**
   ```bash
   curl -sL "https://github.com/martinstark/ralph-rs/archive/vX.Y.Z.tar.gz" | sha256sum
   ```

5. **Update PKGBUILD**
   - Set `pkgver=X.Y.Z`
   - Update `sha256sums` with new checksum

6. **Regenerate .SRCINFO and test build**
   ```bash
   makepkg --printsrcinfo > .SRCINFO
   makepkg -sf
   ```

7. **Push to AUR**
   ```bash
   git add PKGBUILD .SRCINFO
   git commit -m "chore: update to vX.Y.Z"
   git push origin HEAD:master
   ```
