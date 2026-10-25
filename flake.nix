{
  description = "sway-mcp / dms-mcp — MCP servers for Sway and DankMaterialShell IPC control";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = {
    self,
    nixpkgs,
    flake-utils,
  }:
    flake-utils.lib.eachDefaultSystem (
      system: let
        pkgs = import nixpkgs {inherit system;};

        # Build one MCP server binary. The crate builds both binaries; each
        # package installs only its own.
        mkServer = bin:
          pkgs.rustPlatform.buildRustPackage {
            pname = bin;
            version = "0.0.2";
            src = ./.;
            cargoLock.lockFile = ./Cargo.lock;
            cargoBuildFlags = [
              "--bin"
              bin
            ];
            installPhase = ''
              runHook preInstall
              install -Dm755 "$(find target -name '${bin}' -type f -executable | head -n1)" "$out/bin/${bin}"
              runHook postInstall
            '';
            meta = {
              description = "MCP server for Sway/DMS IPC control (${bin})";
              license = pkgs.lib.licenses.mit;
              mainProgram = bin;
            };
          };
      in {
        packages = {
          sway-mcp = mkServer "sway-mcp";
          dms-mcp = mkServer "dms-mcp";
          default = pkgs.symlinkJoin {
            name = "mcp-servers";
            paths = [
              self.packages.${system}.sway-mcp
              self.packages.${system}.dms-mcp
            ];
          };
        };

        devShells.default = pkgs.mkShell {
          packages = with pkgs; [
            cargo
            rustc
            rustfmt
            clippy
            rust-analyzer
            just
          ];
        };
      }
    )
    // {
      # nixpkgs overlay: `nixpkgs.overlays = [ inputs.sway-mcp.overlays.default ]`
      overlays.default = _final: prev: {
        sway-mcp = self.packages.${prev.system}.sway-mcp;
        dms-mcp = self.packages.${prev.system}.dms-mcp;
      };

      # NixOS module:
      #   imports = [ inputs.sway-mcp.nixosModules.default ];
      #   programs.sway-mcp.enable = true;   # and/or programs.dms-mcp.enable
      nixosModules.default = {
        config,
        lib,
        pkgs,
        ...
      }: let
        cfg = config.programs;
      in {
        options.programs = {
          sway-mcp.enable = lib.mkEnableOption "sway-mcp MCP server";
          dms-mcp.enable = lib.mkEnableOption "dms-mcp MCP server";
        };
        config = {
          environment.systemPackages =
            lib.optional cfg.sway-mcp.enable self.packages.${pkgs.system}.sway-mcp
            ++ lib.optional cfg.dms-mcp.enable self.packages.${pkgs.system}.dms-mcp;
        };
      };

      # home-manager module:
      #   imports = [ inputs.sway-mcp.homeManagerModules.default ];
      #   programs.sway-mcp.enable = true;   # and/or programs.dms-mcp.enable
      homeManagerModules.default = {
        config,
        lib,
        pkgs,
        ...
      }: let
        cfg = config.programs;
      in {
        options.programs = {
          sway-mcp.enable = lib.mkEnableOption "sway-mcp MCP server";
          dms-mcp.enable = lib.mkEnableOption "dms-mcp MCP server";
        };
        config = {
          home.packages =
            lib.optional cfg.sway-mcp.enable self.packages.${pkgs.system}.sway-mcp
            ++ lib.optional cfg.dms-mcp.enable self.packages.${pkgs.system}.dms-mcp;
        };
      };
    };
}
