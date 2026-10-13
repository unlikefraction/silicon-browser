import { Button, type ButtonProps } from '@/components/silicon-ui/button/button';
import { useWorkspace } from '../workspace/WorkspaceContext';

export interface TaskButtonProps extends Omit<ButtonProps, 'type' | 'onClick'> {
  /** Runs through `perform`: single-flight, clears the notice, failures become the notice. */
  task: () => Promise<unknown>;
}

/** A `type="button"` Silicon Button that runs a workspace task and is disabled while the workspace is busy. Defaults to `secondary`. */
export function TaskButton({ task, variant = 'secondary', disabled, ...props }: TaskButtonProps) {
  const { busy, action } = useWorkspace();
  return <Button {...props} type="button" variant={variant} disabled={busy || disabled} onClick={action(task)} />;
}

/** A `type="submit"` Silicon Button, disabled while the workspace is busy. Defaults to `primary`. */
export function SubmitButton({ variant = 'primary', disabled, ...props }: Omit<ButtonProps, 'type'>) {
  const { busy } = useWorkspace();
  return <Button {...props} type="submit" variant={variant} disabled={busy || disabled} />;
}
