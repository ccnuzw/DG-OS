// A controllable stand-in for `Channel` of @tauri-apps/api/core. The chat store creates one per send/regenerate and
// hands it to `invoke` as `onEvent`; a test finds it through `MockChannel.instances` or from the invoke arguments and
// calls `emit` at the moment it wants an event to arrive (before or after invoke resolves).
export class MockChannel<T = unknown> {
  static instances: MockChannel[] = [];

  onmessage: (event: T) => void = () => {};

  constructor() {
    MockChannel.instances.push(this as MockChannel);
  }

  emit(event: T): void {
    this.onmessage(event);
  }

  static reset(): void {
    MockChannel.instances = [];
  }

  static last(): MockChannel {
    const channel = MockChannel.instances[MockChannel.instances.length - 1];
    if (!channel) throw new Error('no Channel was created');
    return channel;
  }
}
