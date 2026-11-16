export interface Resource<T> {
  readonly data: T | undefined;
  readonly error: unknown;
  /** True from the moment a load starts until it settles. */
  readonly loading: boolean;
}

/**
 * Loads data and reloads it when the reactive values read at the start of `load` change.
 * A response that arrives after a newer load started is ignored.
 */
export function resource<T>(load: () => Promise<T>): Resource<T> {
  let data = $state<T | undefined>(undefined);
  let error = $state<unknown>(undefined);
  let loading = $state(true);

  $effect(() => {
    let current = true;

    loading = true;

    load().then(
      (value) => {
        if (current) {
          data = value;
          error = undefined;
          loading = false;
        }
      },
      (reason: unknown) => {
        if (current) {
          error = reason;
          loading = false;
        }
      },
    );

    return () => {
      current = false;
    };
  });

  return {
    get data() {
      return data;
    },
    get error() {
      return error;
    },
    get loading() {
      return loading;
    },
  };
}
