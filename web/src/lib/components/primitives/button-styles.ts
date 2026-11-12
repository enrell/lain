export type ButtonVariant = 'primary' | 'secondary' | 'ghost' | 'danger';
export type ButtonSize = 'sm' | 'md' | 'lg' | 'icon';

const base =
	'inline-flex items-center justify-center gap-2 whitespace-nowrap rounded-md font-medium transition-colors duration-150 select-none disabled:opacity-50 disabled:pointer-events-none';

const variants: Record<ButtonVariant, string> = {
	primary: 'bg-accent text-accent-fg hover:bg-accent-hover',
	secondary: 'border border-line bg-surface-active text-foreground hover:bg-surface-hover',
	ghost: 'text-muted hover:bg-surface-hover hover:text-foreground',
	danger: 'border border-danger/30 bg-danger/15 text-danger hover:bg-danger/25'
};

const sizes: Record<ButtonSize, string> = {
	// Phones get thumb-sized targets; from md up the compact desktop sizes.
	sm: 'h-10 px-3.5 text-sm md:h-8 md:px-3',
	md: 'h-11 px-4 text-sm md:h-10',
	lg: 'h-12 px-6 text-base',
	icon: 'size-11 md:size-9'
};

export function buttonClasses(
	variant: ButtonVariant = 'primary',
	size: ButtonSize = 'md',
	className = ''
): string {
	return [base, variants[variant], sizes[size], className].join(' ');
}
