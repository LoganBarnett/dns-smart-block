# Example NixOS configuration for DNS Smart Block
#
# This shows how to use the dns-smart-block NixOS module in your system configuration
#
{ config, pkgs, ... }:

{
  # Import the dns-smart-block module
  # In a real configuration, you'd import it from the flake:
  # imports = [ inputs.dns-smart-block.nixosModules.default ];

  services.dns-smart-block = {
    enable = true;

    # NATS Configuration
    # The module includes a built-in NATS server that won't conflict with other instances
    nats = {
      enable = true;
      port = 4222;  # Change if you have another NATS instance
      subject = "dns.smart-block.domains";  # Unique subject for this service
    };

    # Log Processor - watches dnsdist logs
    logProcessor = {
      enable = true;

      # Using journalctl to watch dnsdist logs (recommended)
      logSource = "cmd:journalctl -f -u dnsdist";

      # Or watch a log file directly:
      # logSource = "/var/log/dnsdist.log";

      # Optional: skip checking dnsdist for already-blocked domains
      skipDnsdistCheck = false;
    };

    # Queue Processor - classifies domains
    queueProcessor = {
      enable = true;
      httpTimeoutSec = 10;
      httpMaxKb = 100;
      minConfidence = 0.8;  # Only block if LLM is 80% confident
    };

    # Ollama LLM Configuration
    ollama = {
      url = "http://localhost:11434";
      model = "llama2";  # or "llama3", "mistral", etc.
    };

    # dnsdist API Configuration
    dnsdist = {
      apiUrl = "http://localhost:8080";

      # Store API key in a file for security
      # apiKeyFile = "/run/secrets/dnsdist-api-key";
      apiKeyFile = null;  # Set if dnsdist requires authentication
    };

    # Classifier Configuration
    # Option 1: Use the bundled gaming classifier (recommended for getting started)
    classifier.preset = "gaming";

    # Option 2: Provide your own custom classifier prompt
    # classifier.customTemplate = pkgs.writeText "custom-prompt.txt" ''
    #   You are classifying websites for social media content.
    #
    #   Analyze this metadata: {{INPUT_JSON}}
    #
    #   Respond with JSON:
    #   {
    #     "is_matching_site": true or false,
    #     "confidence": 0.0 to 1.0
    #   }
    # '';
  };

  # Ensure Ollama is running (if using local Ollama)
  # services.ollama = {
  #   enable = true;
  #   acceleration = "cuda";  # or "rocm" for AMD
  # };

  # Ensure dnsdist is configured
  # services.dnsdist = {
  #   enable = true;
  #   # ... your dnsdist configuration
  # };
}
