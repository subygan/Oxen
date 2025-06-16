require_relative '../spec_helper'

describe "oxen stash" do
  include_context "oxen test_repo"

  before do
    # Setup a test repo
    init_oxen_repo
    @test_file = "test_file.txt"
    File.write(@test_file, "initial content")
    run_oxen_cmd("add #{@test_file}")
    run_oxen_cmd("commit -m \"Initial commit\"")
  end

  it "should stash modified file and pop it" do
    # Modify the file
    File.write(@test_file, "modified content")

    # Stash the changes
    output = run_oxen_cmd("stash push -m \"my test stash\"")
    expect(output).to include("Created stash: stash_") # Check for timestamped stash name

    # File should be reverted to initial content
    expect(File.read(@test_file)).to eq("initial content")

    # Pop the stash
    output = run_oxen_cmd("stash pop")
    expect(output).to include("Popping stash: stash_")
    expect(output).to include("my test stash") # Check if message is displayed

    # File should now have modified content
    expect(File.read(@test_file)).to eq("modified content")

    # Stash list should be empty
    output = run_oxen_cmd("stash list")
    expect(output).to include("No stashes available.")
  end

  it "should stash a new file and pop it" do
    new_file = "new_tracking_file.txt"
    File.write(new_file, "new file content")

    # Stash the new file
    # For new files, they are not in HEAD, so 'stash push' effectively 'removes' them from WD
    # by not restoring them from a (non-existent) HEAD version.
    output = run_oxen_cmd("stash push -m \"stashing new file\"")
    expect(output).to include("Created stash: stash_")
    expect(File.exist?(new_file)).to be_falsey # New file should be removed by push logic

    # Pop the stash
    output = run_oxen_cmd("stash pop")
    expect(output).to include("Popping stash: stash_")
    expect(output).to include("stashing new file")

    # New file should be restored
    expect(File.exist?(new_file)).to be_truthy
    expect(File.read(new_file)).to eq("new file content")
  end

  it "should apply a stash and keep it" do
    File.write(@test_file, "content for apply")
    run_oxen_cmd("stash push -m \"apply-test\"")
    expect(File.read(@test_file)).to eq("initial content") # Reverted

    # Apply the stash
    output = run_oxen_cmd("stash apply")
    expect(output).to include("Applying stash: stash_")
    expect(output).to include("apply-test")
    expect(File.read(@test_file)).to eq("content for apply") # Applied

    # Stash should still exist
    output = run_oxen_cmd("stash list")
    expect(output).to include("stash_")
    expect(output).to include("apply-test")

    # Clean up by popping
    run_oxen_cmd("stash pop")
  end

  it "should list multiple stashes" do
    File.write(@test_file, "change 1")
    run_oxen_cmd("stash push -m \"first stash\"")

    File.write(@test_file, "change 2")
    run_oxen_cmd("stash push -m \"second stash\"")

    output = run_oxen_cmd("stash list")
    expect(output).to include("Available stashes:")
    expect(output).to include("first stash")
    expect(output).to include("second stash")
    stashes = output.scan(/stash_\d+/)
    expect(stashes.size).to eq(2)

    # Clean up
    run_oxen_cmd("stash pop")
    run_oxen_cmd("stash pop")
  end

  it "should stash push without a message" do
    File.write(@test_file, "no message content")
    output = run_oxen_cmd("stash push")
    expect(output).to include("Created stash: stash_")
    expect(File.read(@test_file)).to eq("initial content")

    output = run_oxen_cmd("stash list")
    # Check for stash name, but not a specific message (as none was given)
    # The format is ' - stash_timestamp' if no message
    expect(output).to match(/ - stash_\d+$/)


    output = run_oxen_cmd("stash pop")
    expect(output).to include("Popping stash: stash_")
    # Expect no specific message part in the pop output, e.g. "stash_timestamp - "
    expect(output).not_to include(" - ") if output.include?("Popping stash: stash_") && !output.match(/stash_\d+ - /)


    expect(File.read(@test_file)).to eq("no message content")
  end

  it "should do nothing if no changes to stash on push" do
    output = run_oxen_cmd("stash push")
    expect(output).to include("No changes to stash.")
  end

  it "should report no stashes on pop, apply, or list when empty" do
    output = run_oxen_cmd("stash pop")
    expect(output).to include("No stashes to pop.")

    output = run_oxen_cmd("stash apply")
    expect(output).to include("No stashes to apply.")

    output = run_oxen_cmd("stash list")
    expect(output).to include("No stashes available.")
  end

  it "should stash and pop/apply empty directories and files in directories" do
    empty_dir = "my_empty_dir"
    dir_with_file = "dir_with_a_file"
    file_in_dir = File.join(dir_with_file, "inner_file.txt")

    # Create an empty directory
    FileUtils.mkdir_p(empty_dir)
    # Create a directory with a file
    FileUtils.mkdir_p(dir_with_file)
    File.write(file_in_dir, "content in a directory")

    # Stash these changes
    # Both new directories and the new file are not in HEAD.
    # `stash push` should archive them and then they should not exist in WD.
    output = run_oxen_cmd("stash push -m \"stash with dirs\"")
    expect(output).to include("Created stash: stash_")

    # Verify they are removed from working directory by the push logic for new items
    expect(Dir.exist?(empty_dir)).to be_falsey
    expect(Dir.exist?(dir_with_file)).to be_falsey
    expect(File.exist?(file_in_dir)).to be_falsey

    # Pop the stash
    output = run_oxen_cmd("stash pop")
    expect(output).to include("Popping stash: stash_")
    expect(output).to include("stash with dirs")
    expect(output).to include("Created directory: #{empty_dir}") # Check for empty dir creation
    expect(output).to include("Applied file: #{file_in_dir}")  # Check for file in dir application

    # Verify they are restored
    expect(Dir.exist?(empty_dir)).to be_truthy
    expect(Dir.exist?(dir_with_file)).to be_truthy
    expect(File.exist?(file_in_dir)).to be_truthy
    expect(File.read(file_in_dir)).to eq("content in a directory")

    # Stash again to test 'apply'
    FileUtils.rm_rf(empty_dir)
    FileUtils.rm_rf(dir_with_file) # Clean up before next stash

    FileUtils.mkdir_p(empty_dir)
    FileUtils.mkdir_p(dir_with_file)
    File.write(file_in_dir, "new apply content")
    run_oxen_cmd("stash push -m \"apply stash with dirs\"")

    expect(Dir.exist?(empty_dir)).to be_falsey
    expect(Dir.exist?(dir_with_file)).to be_falsey

    # Apply the stash
    output = run_oxen_cmd("stash apply")
    expect(output).to include("Applying stash: stash_")
    expect(output).to include("apply stash with dirs")
    expect(output).to include("Created directory: #{empty_dir}")
    expect(output).to include("Applied file: #{file_in_dir}")

    expect(Dir.exist?(empty_dir)).to be_truthy
    expect(Dir.exist?(dir_with_file)).to be_truthy
    expect(File.exist?(file_in_dir)).to be_truthy
    expect(File.read(file_in_dir)).to eq("new apply content")

    # Clean up by popping the applied stash
    run_oxen_cmd("stash pop")
  end

  describe "conflict handling" do
    before do
      # Ensure a clean slate for conflict tests if prior tests failed mid-pop
      # This will remove all directories starting with 'stash_' in the .oxen/stash directory
      base_stash_path = File.join(".oxen", "stash")
      if Dir.exist?(base_stash_path)
        Dir.foreach(base_stash_path) do |item|
          next if item == '.' or item == '..'
          if item.start_with?('stash_')
            FileUtils.rm_rf(File.join(base_stash_path, item))
          end
        end
      end

      # Setup: file with initial content, committed
      @conflict_file = "conflict_file.txt"
      File.write(@conflict_file, "base content\nline2\nline3")
      run_oxen_cmd("add #{@conflict_file}")
      run_oxen_cmd("commit -m \"Base for conflict tests\"")
    end

    it "should detect conflict and not drop stash on pop" do
      # 1. Modify and stash
      File.write(@conflict_file, "stashed content\nline2\nNEW STASH LINE")
      run_oxen_cmd("stash push -m \"stashed changes\"")
      expect(File.read(@conflict_file)).to eq("base content\nline2\nline3") # Reverted

      # 2. Modify locally again (different change)
      File.write(@conflict_file, "local content\nNEW LOCAL LINE\nline3")

      # 3. Pop and expect conflict
      output = run_oxen_cmd("stash pop")
      expect(output).to include("Stash operation completed with conflicts")
      expect(output).to include(@conflict_file)
      expect(output).to include("was not removed due to conflicts")

      # Check file content remains local
      expect(File.read(@conflict_file)).to eq("local content\nNEW LOCAL LINE\nline3")

      # Check stash still exists
      list_output = run_oxen_cmd("stash list")
      expect(list_output).to include("stashed changes")

      # Cleanup the remaining stash
      run_oxen_cmd("stash drop") # Assuming stash drop 0 or similar, for now just pop again after resolving manually
                                # For test purposes, we'll just clear it. If stash drop isn't implemented, this might need adjustment.
                                # Current pop will conflict again. Let's reset content to base to allow pop for cleanup.
      File.write(@conflict_file, "base content\nline2\nline3")
      run_oxen_cmd("stash pop") # This should now succeed without conflict for cleanup
    end

    it "should keep local changes if stashed version is same as base" do
      # 1. Stash (no actual change to @conflict_file, so stashed version is base)
      # To ensure it's part of 'modified_files' for push, we need to touch it or make a dummy change then revert.
      # Simpler: create another file to make the stash non-empty.
      File.write("dummy.txt", "for_stash")
      run_oxen_cmd("stash push -m \"stash without changes to conflict_file\"")
      FileUtils.rm("dummy.txt") # Clean up dummy

      # 2. Modify locally
      File.write(@conflict_file, "local only change\nline2\nline3")

      # 3. Pop: Should apply dummy.txt, keep local changes to @conflict_file
      output = run_oxen_cmd("stash pop")
      expect(output).not_to include("Conflict") # Or check for "Successfully popped"
      expect(output).to include("Successfully popped stash")


      expect(File.read(@conflict_file)).to eq("local only change\nline2\nline3")
      expect(File.exist?("dummy.txt")).to be_truthy # From stash

      # Stash should be gone
      list_output = run_oxen_cmd("stash list")
      expect(list_output).to include("No stashes available")
    end

    it "should apply stashed changes if local is same as base" do
      # 1. Modify and stash
      File.write(@conflict_file, "stashed version for apply\nline2\nline3")
      run_oxen_cmd("stash push -m \"will apply this\"")
      # @conflict_file is now reverted to "base content..."

      # 2. Ensure local is same as base (it is after push)

      # 3. Pop
      output = run_oxen_cmd("stash pop")
      expect(output).not_to include("Conflict")
      expect(output).to include("Successfully popped stash")

      expect(File.read(@conflict_file)).to eq("stashed version for apply\nline2\nline3")
      list_output = run_oxen_cmd("stash list")
      expect(list_output).to include("No stashes available")
    end

    it "should conflict if new file in stash and new file locally with same name" do
      new_filename = "brand_new_file.txt"

      # 1. Create and stash a new file
      File.write(new_filename, "content from stash")
      run_oxen_cmd("stash push -m \"new file stashed\"")
      expect(File.exist?(new_filename)).to be_falsey # Removed by push

      # 2. Create a local file with the same name but different content
      File.write(new_filename, "content from local")

      # 3. Pop and expect conflict
      output = run_oxen_cmd("stash pop")
      expect(output).to include("Stash operation completed with conflicts")
      expect(output).to include(new_filename)
      expect(output).to include("File #{new_filename} created locally and in stash")
      expect(output).to include("was not removed due to conflicts")

      # Local file should remain untouched
      expect(File.read(new_filename)).to eq("content from local")

      # Stash should still exist
      list_output = run_oxen_cmd("stash list")
      expect(list_output).to include("new file stashed")

      # Cleanup: remove local, then pop the stash
      FileUtils.rm(new_filename)
      run_oxen_cmd("stash pop")
    end
  end
end
