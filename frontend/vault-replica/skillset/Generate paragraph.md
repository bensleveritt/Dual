---
- command: Generate a paragraph based on "something"
- command: Come up with a paragraph based on "some text"
- command: Generate one paragraph based on "This is the answer:"
- result: "#1"
---

```js
(async () => {
	const rawResponse = await fetch("http://127.0.0.1:5000/generate/", {
		"method": "POST",
		"headers": {
			"Accept": "application/json",
			"Content-Type": "application/json",
		},
		"body": JSON.stringify({
			"prompt": `*quoted content*`,
			"behavior": "finish_paragraph"
		})
	});

	var content = await rawResponse.json();
	return content["result"][0];
})();
```