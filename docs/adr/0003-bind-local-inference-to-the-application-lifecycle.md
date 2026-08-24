# Bind local inference to the application lifecycle

The macOS application will own the local oMLX service and model storage lifecycle. Opening the application starts the service, idle-time policy may unload the Qwen model while leaving the service available, and closing the application stops the service. Più stores the downloaded model in one fixed application-managed directory rather than exposing a model-location setting. This keeps the product operationally equivalent to one application without embedding model inference into the agent runtime or frontend.
