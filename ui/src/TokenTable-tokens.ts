export const YOU_ID = "you";
const tokenFormatter = new Intl.NumberFormat("en-US");

export function formatTokens(value: number): string {
	return `${tokenFormatter.format(value)} tokens`;
}
