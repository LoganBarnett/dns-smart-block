################################################################################
# Prometheus exporter for dns-smart-block.
#
# This module defines the exporter configuration that allows Prometheus to
# scrape dns-smart-block metrics. The actual metrics endpoint is provided by
# dns-smart-block-blocklist-server, so this module only handles the exporter
# interface and firewall configuration.
################################################################################
{ config, lib, ... }: let
  cfg = config.services.prometheus.exporters.dns-smart-block-exporter;
in {
  options.services.prometheus.exporters.dns-smart-block-exporter = {
    enable = lib.mkEnableOption "Prometheus dns-smart-block exporter";

    port = lib.mkOption {
      type = lib.types.port;
      default = 3000;
      description = "Port for the dns-smart-block metrics endpoint";
    };

    openFirewall = lib.mkOption {
      type = lib.types.bool;
      default = false;
      description = "Whether to open the firewall for the exporter port";
    };
  };

  config = lib.mkIf cfg.enable {
    # Open firewall port if requested.
    networking.firewall.allowedTCPPorts = lib.mkIf cfg.openFirewall [ cfg.port ];
  };
}
