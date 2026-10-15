# NixOS module for Neori.
#
# Mirrors nixpkgs' `programs.niri` module
# (nixos/modules/programs/wayland/niri.nix), adapted for Neori:
#
#   * the package is injected by the flake (`neoriPackage`), so the module works even
#     though `neori` is not in nixpkgs;
#   * the compositor, session, systemd units and portals are named `neori`.
#
# The `wayland-session.nix` bits are inlined below with `enableWlrPortal = false` and
# `enableXWayland = false`, exactly like the niri module calls it.
{ neoriPackage }:
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.programs.neori;
in
{
  options.programs.neori = {
    enable = lib.mkEnableOption "Neori, a scrollable-tiling Wayland compositor with extended features";

    package = lib.mkOption {
      type = lib.types.package;
      default = neoriPackage;
      defaultText = lib.literalExpression "neori.packages.\${pkgs.stdenv.hostPlatform.system}.neori";
      description = "The neori package to use.";
    };

    useNautilus = lib.mkEnableOption "Nautilus as file-chooser for xdg-desktop-portal-gnome" // {
      default = true;
    };
  };

  config = lib.mkIf cfg.enable {
    environment.systemPackages = [
      cfg.package
    ];

    # Required for xdg-desktop-portal-gnome's FileChooser to work properly.
    services.dbus.packages = lib.mkIf cfg.useNautilus [
      pkgs.nautilus
    ];

    services = {
      displayManager.sessionPackages = [ cfg.package ];

      # Recommended by upstream
      # https://github.com/YaLTeR/niri/wiki/Important-Software#portals
      gnome.gnome-keyring.enable = lib.mkDefault true;

      # From nixpkgs' programs/wayland/wayland-session.nix.
      graphical-desktop.enable = true;
    };

    systemd.packages = [ cfg.package ];

    # Restarting the compositor kills the graphical session; same treatment as the
    # display-manager modules.
    systemd.user.services.neori = {
      restartIfChanged = false;
      # Defining the unit here generates a drop-in; without this it would carry the
      # NixOS default Environment="PATH=coreutils:…", clobbering the PATH that
      # neori-session imported into the user manager and breaking spawn actions that
      # rely on it.
      enableDefaultPath = false;
    };

    xdg.portal = {
      enable = lib.mkDefault true;

      # NOTE: `configPackages` is ignored when `xdg.portal.config.neori` is defined.
      config.neori = {
        default = [
          "gnome"
          "gtk"
        ];
        "org.freedesktop.impl.portal.Access" = "gtk";
        "org.freedesktop.impl.portal.FileChooser" = lib.mkIf (!cfg.useNautilus) "gtk";
        "org.freedesktop.impl.portal.Notification" = "gtk";
        "org.freedesktop.impl.portal.Secret" = "gnome-keyring";
      };

      # Recommended by upstream, required for screencast support.
      extraPortals = [
        pkgs.xdg-desktop-portal-gnome
        # From wayland-session.nix (enableGtkPortal defaults to true).
        pkgs.xdg-desktop-portal-gtk
      ];
    };

    # From wayland-session.nix.
    security = {
      polkit.enable = true;
      pam.services.swaylock = { };
    };

    programs.dconf.enable = lib.mkDefault true;

    # Window manager only sessions (unlike DEs) don't handle XDG autostart files, so
    # force them to run the service.
    services.xserver.desktopManager.runXdgAutostartIfNone = lib.mkDefault true;
  };
}
