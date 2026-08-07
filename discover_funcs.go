func discoverTraecliModels(ctx context.Context, executablePath string) ([]Model, error) {
	return discoverACPModels(ctx, executablePath, acpDiscoveryProvider{
		defaultBin:   "traecli",
		clientName:   "multica-model-discovery",
		tmpdirPrefix: "multica-traecli-discovery-",
		acpArgs:      []string{"acp", "serve", "--yolo"},
	})
}


func discoverOpenCodeModels(ctx context.Context, executablePath string) ([]Model, error) {
	if executablePath == "" {
		executablePath = "opencode"
	}


func discoverPiModels(ctx context.Context, executablePath string) ([]Model, error) {
	if executablePath == "" {
		executablePath = "pi"
	}


func discoverHermesModels(ctx context.Context, executablePath string) ([]Model, error) {
	return discoverACPModels(ctx, executablePath, acpDiscoveryProvider{
		defaultBin:   "hermes",
		clientName:   "multica-model-discovery",
		extraEnv:     []string{"HERMES_YOLO_MODE=1"},
		tmpdirPrefix: "multica-hermes-discovery-",
	})
}


func discoverKimiModels(ctx context.Context, executablePath string) ([]Model, error) {
	return discoverACPModels(ctx, executablePath, acpDiscoveryProvider{
		defaultBin:   "kimi",
		clientName:   "multica-model-discovery",
		tmpdirPrefix: "multica-kimi-discovery-",
	})
}


func discoverKiroModels(ctx context.Context, executablePath string) ([]Model, error) {
	return discoverACPModels(ctx, executablePath, acpDiscoveryProvider{
		defaultBin:   "kiro-cli",
		clientName:   "multica-model-discovery",
		tmpdirPrefix: "multica-kiro-discovery-",
	})
}


func discoverCopilotModels(ctx context.Context, executablePath string) ([]Model, error) {
	models, err := discoverACPModels(ctx, executablePath, acpDiscoveryProvider{
		defaultBin:   "copilot",
		clientName:   "multica-model-discovery",
		tmpdirPrefix: "multica-copilot-discovery-",
		acpArgs:      []string{"--acp"},
	})
	if err != nil || len(models) == 0 {
		return copilotStaticModels(), nil
	}


func discoverQoderModels(ctx context.Context, executablePath string) ([]Model, error) {
	return discoverACPModels(ctx, executablePath, acpDiscoveryProvider{
		defaultBin:   "qodercli",
		clientName:   "multica-model-discovery",
		acpArgs:      []string{"--yolo", "--acp"},
		tmpdirPrefix: "multica-qoder-discovery-",
	})
}


func discoverACPModels(ctx context.Context, executablePath string, p acpDiscoveryProvider) ([]Model, error) {
	fail := func(stage string, err error) ([]Model, error) {
		if p.strictErrors {
			return nil, fmt.Errorf("ACP model discovery %s failed: %w", stage, err)
		}


func discoverAntigravityModels(ctx context.Context, executablePath string) ([]Model, error) {
	if executablePath == "" {
		executablePath = "agy"
	}


func discoverGrokModels(ctx context.Context, executablePath string) ([]Model, error) {
	// Match the daemon's runtime launch: `--no-auto-update` (global) so a
	// background update check can't stall discovery. Auth is selected only
	// after initialize returns the methods this installed CLI actually offers.
	models, err := discoverACPModels(ctx, executablePath, acpDiscoveryProvider{
		defaultBin:   "grok",
		clientName:   "multica-model-discovery",
		tmpdirPrefix: "multica-grok-discovery-",
		acpArgs:      []string{"--no-auto-update", "agent", "--always-approve", "stdio"},
		selectAuthMethod: func(initResult json.RawMessage, childEnv []string) (string, error) {
			return selectGrokAuthMethod(extractACPAuthMethods(initResult), envHasNonEmpty(childEnv, "XAI_API_KEY"))
		},
		strictErrors: true,
	})
	if err != nil || len(models) == 0 {
		if err != nil {
			slog.Debug("grok model discovery fell back to static catalog", "error", err)
		}


func discoverCursorModels(ctx context.Context, executablePath string) ([]Model, error) {
	if executablePath == "" {
		executablePath = "cursor-agent"
	}


func discoverOpenclawAgents(ctx context.Context, executablePath string) ([]Model, error) {
	if executablePath == "" {
		executablePath = "openclaw"
	}


func discoverCodebuddyModels(ctx context.Context, executablePath string) ([]Model, error) {
	if executablePath == "" {
		executablePath = "codebuddy"
	}


