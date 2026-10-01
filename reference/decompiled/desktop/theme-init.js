;(function () {
	const mediaQuery = window.matchMedia("(prefers-color-scheme: dark)")
	/**
	 * @deprecated Use the data-framer-theme attribute instead. Can be
	 * removed after the code block module has been updated.
	 */
	document.body.classList.toggle("framer-theme-dark", mediaQuery.matches)
	document.body.dataset.framerTheme = mediaQuery.matches ? "dark" : "light"
})()
