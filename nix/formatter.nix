# Code formatter for the repository: `nix fmt` formats every file, and
# `nix flake check` checks that every file is formatted.
{
  pkgs,
  inputs,
  flake,
  ...
}: let
  treefmt = inputs.treefmt-nix.lib.evalModule pkgs {
    projectRootFile = "flake.nix";

    # Nix.
    programs.alejandra.enable = true;

    # Rust, for the edition the workspace uses.
    programs.rustfmt = {
      enable = true;
      edition = "2024";
    };

    # TOML, ordered and validated with the JSON Schemas that tombi ships
    # (Cargo.toml, lakefile.toml, …). Offline, so that it also runs in the
    # build sandbox.
    settings.formatter.tombi = {
      command = pkgs.lib.getExe pkgs.tombi;
      options = ["format" "--offline"];
      includes = ["*.toml"];
    };

    # The Justfile.
    programs.just.enable = true;

    # The web app: TypeScript, Svelte, CSS and JSON, as `biome.json` says.
    # The same version as the web app's own, so both agree.
    settings.formatter.biome = {
      command = pkgs.lib.getExe pkgs.biome;
      options = ["format" "--write" "--no-errors-on-unmatched"];
      includes = [
        "web/*.json"
        "web/*.ts"
        "web/scripts/*.mjs"
        "web/src/*.ts"
        "web/src/**/*.ts"
        "web/src/*.svelte"
        "web/src/**/*.svelte"
        "web/src/*.css"
        "web/src/**/*.css"
        "web/src/*.html"
        "web/src/**/*.html"
        "desktop/*.json"
        "desktop/*.ts"
        "desktop/*.html"
        "desktop/src/*.ts"
        "desktop/src/**/*.ts"
        "desktop/src/*.svelte"
        "desktop/src/**/*.svelte"
        "desktop/src/*.css"
        "desktop/src/**/*.css"
        "desktop/src-tauri/*.json"
        "desktop/src-tauri/capabilities/*.json"
        "biome.json"
      ];
    };
  };
in
  treefmt.config.build.wrapper.overrideAttrs (old: {
    passthru =
      old.passthru
      // {
        tests.check = treefmt.config.build.check flake;
      };
  })
