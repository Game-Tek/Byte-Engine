import { loader } from 'fumadocs-core/source';
import {
	IconAppStack,
	IconBookOpen,
	IconBolt,
	IconBox,
	IconBoxArchive,
	IconBrightnessIncrease,
	IconBug,
	IconClipboardCheck,
	IconCodeEditor,
	IconColorPalette,
	IconComputerDownload,
	IconConnect,
	IconFlag,
	IconFolderContent,
	IconGear,
	IconHammer,
	IconJoystickCross,
	IconMonitor,
	IconMsgs,
	IconRocket,
	IconRulerPen,
	IconSparkle,
	IconStorage,
	IconWindow,
	IconWrenchScrewdriver,
} from 'nucleo-glass';
import { createElement, type ComponentType } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { docs } from '@/.source/server';
import { inspectorOpenAPI } from '@/lib/openapi';

type GlassIcon = ComponentType<{
	'aria-hidden': true;
	className: string;
	uniqueId?: string;
}>;

// Render icons to strings because the page tree crosses the server/client boundary.
const icons = {
	Binary: IconCodeEditor,
	BookOpen: IconBookOpen,
	Book: IconBookOpen,
	Boxes: IconAppStack,
	Braces: IconCodeEditor,
	Bug: IconBug,
	Cable: IconConnect,
	CodeEditor: IconCodeEditor,
	Computer: IconMonitor,
	Cpu: IconRulerPen,
	Database: IconStorage,
	Download: IconComputerDownload,
	Flag: IconFlag,
	FolderPlus: IconFolderContent,
	Gamepad2: IconJoystickCross,
	Package: IconBox,
	PackageCheck: IconClipboardCheck,
	PackageOpen: IconBoxArchive,
	PanelTop: IconWindow,
	MessagesSquare: IconMsgs,
	Hammer: IconHammer,
	JoystickCross: IconJoystickCross,
	Rocket: IconRocket,
	RulerPen: IconRulerPen,
	Settings: IconGear,
	Sparkles: IconSparkle,
	SquareFunction: IconBolt,
	Storage: IconStorage,
	Sun: IconBrightnessIncrease,
	SwatchBook: IconColorPalette,
	TerminalSquare: IconCodeEditor,
	Wrench: IconWrenchScrewdriver,
	WrenchScrewdriver: IconWrenchScrewdriver,
} satisfies Record<string, GlassIcon>;

export const source = loader({
	source: docs.toFumadocsSource(),
	baseUrl: '/docs',
	plugins: [inspectorOpenAPI.loaderPlugin()],
	icon(icon) {
		if (!icon) {
			return;
		}

		if (!(icon in icons)) return;

		const Icon = icons[icon as keyof typeof icons];
		return renderToStaticMarkup(
			createElement(Icon, {
				'aria-hidden': true,
				className: 'be-glass-icon',
				uniqueId: `be-${icon.toLowerCase()}-`,
			}),
		);
	},
});
