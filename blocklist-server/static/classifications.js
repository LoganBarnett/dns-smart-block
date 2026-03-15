let sortDirection = {};

function sortTable(columnIndex) {
  const table = document.getElementById('classificationsTable');
  const tbody = table.querySelector('tbody');
  const rows = Array.from(tbody.querySelectorAll('tr'));

  const currentDirection = sortDirection[columnIndex] || 'asc';
  const newDirection = currentDirection === 'asc' ? 'desc' : 'asc';
  sortDirection = { [columnIndex]: newDirection };

  rows.sort((a, b) => {
    let aValue = a.cells[columnIndex].textContent.trim();
    let bValue = b.cells[columnIndex].textContent.trim();

    // Try to parse as date/time.
    const aDate = new Date(aValue);
    const bDate = new Date(bValue);

    if (!isNaN(aDate.getTime()) && !isNaN(bDate.getTime())) {
      return newDirection === 'asc'
        ? aDate.getTime() - bDate.getTime()
        : bDate.getTime() - aDate.getTime();
    }

    // Try to parse as number.
    const aNum = parseFloat(aValue);
    const bNum = parseFloat(bValue);

    if (!isNaN(aNum) && !isNaN(bNum)) {
      return newDirection === 'asc' ? aNum - bNum : bNum - aNum;
    }

    // String comparison.
    if (newDirection === 'asc') {
      return aValue.localeCompare(bValue);
    } else {
      return bValue.localeCompare(aValue);
    }
  });

  rows.forEach(row => tbody.appendChild(row));

  // Update header indicators.
  table.querySelectorAll('th').forEach((th, idx) => {
    th.classList.remove('sorted-asc', 'sorted-desc');
    if (idx === columnIndex) {
      th.classList.add(`sorted-${newDirection}`);
    }
  });
}

async function expireDomain(domain) {
  const button = event.target;
  button.disabled = true;
  button.textContent = 'Expiring...';

  try {
    const response = await fetch(`/expire?domain=${encodeURIComponent(domain)}`, {
      method: 'POST',
    });

    if (response.ok) {
      const result = await response.text();
      alert(`Success: ${result}`);
      // Reload the page to show updated data
      window.location.reload();
    } else {
      const error = await response.text();
      alert(`Error: ${error}`);
      button.disabled = false;
      button.textContent = 'Expire';
    }
  } catch (error) {
    alert(`Failed to expire domain: ${error.message}`);
    button.disabled = false;
    button.textContent = 'Expire';
  }
}
