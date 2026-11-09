# pagefind as nixpkgs ships it, plus the two inputs its zstd-sys build script
# needs. nixpkgs' pagefind 1.6.0 vendors zstd-sys, whose build.rs probes
# pkg-config for libzstd and panics when pkg-config is absent; the package
# declares neither, so it fails to build and takes the dev shell, every site
# check and a tenant's packages.site down with it. Adding inputs a fixed
# nixpkgs already has is harmless, so this can stay until upstream catches up.
{pkgs}:
pkgs.pagefind.overrideAttrs (old: {
  nativeBuildInputs = (old.nativeBuildInputs or []) ++ [pkgs.pkg-config];
  buildInputs = (old.buildInputs or []) ++ [pkgs.zstd];
})
