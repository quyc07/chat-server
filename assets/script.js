var eventSource = new EventSource('event/sse');

eventSource.onmessage = function(event) {
    console.log('Message from server ', event.data);
}
