import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, test, vi } from 'vitest';
import Modal from './Modal';

afterEach(cleanup);
describe('shared glass modal', () => {
  test('does not create a hidden dialog subtree while closed', () => {
    const { container } = render(<Modal open={false} onClose={vi.fn()}>Content</Modal>);
    expect(container.childElementCount).toBe(0);
  });
  test('preserves content clicks and closes only from the overlay', () => {
    const close = vi.fn();
    render(<Modal open onClose={close} title="Settings"><span>Content</span></Modal>);
    fireEvent.click(screen.getByText('Content'));
    expect(close).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole('dialog'));
    expect(close).toHaveBeenCalledTimes(1);
  });
  test('retains the close action without a full-window blur', () => {
    const close = vi.fn();
    render(<Modal open onClose={close} title="Settings">Content</Modal>);
    expect(screen.getByRole('dialog').className).not.toContain('backdrop-blur');
    fireEvent.click(screen.getByRole('button'));
    expect(close).toHaveBeenCalledTimes(1);
  });
});
